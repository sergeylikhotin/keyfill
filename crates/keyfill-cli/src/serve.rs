//! `keyfill serve`: живые каналы под управлением пульта.
//!
//! У каждого канала свой поток: он владеет сессией (JS не делится между потоками), тикает по
//! часам кадра и на границе кадра применяет пришедшие команды. Сеть — потоки соединений:
//! HTTP (`/schema`, `/state`, `/monitor`) и WebSocket (`/ws`) с командами и событиями. Принятые
//! команды пишутся в журнал по ходу эфира; `--resume` поднимает канал из последнего журнала
//! сразу «в эфир».

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use keyfill_core::{EvalOptions, Frame};
use keyfill_js::RunOptions;
use keyfill_render::{Png, RenderOptions, Renderer};
use serde::Serialize;
use serde_json::{Value, json};
use tungstenite::protocol::Role;
use tungstenite::{Message, WebSocket};

use crate::channel::{Ack, AckStatus, Batch, Entry, Header, Journal, JOURNAL_SUFFIX};
use crate::view::{Request, read_request, respond};
use crate::{Bundle, Session, slash};

pub struct ServeOptions {
    /// Номер канала → файл канала.
    pub channels: BTreeMap<u32, PathBuf>,
    pub host: String,
    pub port: u16,
    /// Токен доступа; без него сервер слушает только loopback.
    pub token: Option<String>,
    /// Каталог журналов: `ch<N>-<время>.journal.jsonl`.
    pub journal_dir: PathBuf,
    /// Поднять каналы из последних журналов сразу «в эфир».
    pub resume: bool,
    pub run: RunOptions,
    pub eval: EvalOptions,
}

/// Loopback-адрес: без токена сервер слушает только его.
fn loopback(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "::1" | "localhost")
}

pub fn serve(mut opts: ServeOptions) -> Result<()> {
    if opts.channels.is_empty() {
        return Err(crate::usage("нет каналов: keyfill serve channel.tsx или channels: { 1: 'channel.tsx' } в keyfill.config.ts"));
    }
    if !loopback(&opts.host) && opts.token.is_none() {
        return Err(crate::usage(format!("--host {}: доступ из сети только с токеном: --token <секрет> или KEYFILL_TOKEN", opts.host)));
    }
    // Часы JS — момент запуска: `new Date()` в сцене показывает его, а журнал записывает,
    // чтобы воспроизведение видело то же самое.
    opts.run.now_ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0);
    std::fs::create_dir_all(&opts.journal_dir).with_context(|| format!("каталог журналов {}", slash(&opts.journal_dir)))?;

    let hub = Hub::default();
    let mut channels = BTreeMap::new();
    for (&n, file) in &opts.channels {
        let bundle = crate::bundle(file)?;
        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let ctx = RunnerContext {
            number: n,
            file: file.clone(),
            bundle,
            run: opts.run.clone(),
            eval: opts.eval,
            journal_dir: opts.journal_dir.clone(),
            resume: opts.resume,
            hub: hub.clone(),
        };
        std::thread::Builder::new().name(format!("channel {n}")).spawn(move || match Runner::start(ctx) {
            Ok(mut runner) => {
                let _ = ready_tx.send(Ok(runner.journal_path.clone()));
                runner.run(rx);
            }
            Err(e) => {
                let _ = ready_tx.send(Err(format!("{e:#}")));
            }
        })?;
        let journal = ready_rx.recv().context("поток канала не запустился")?.map_err(|e| anyhow::anyhow!("канал {n}: {e}"))?;
        eprintln!("keyfill serve: канал {n} — {} · журнал {}", slash(file), slash(&journal));
        channels.insert(n, tx);
    }

    let listener = TcpListener::bind((opts.host.as_str(), opts.port)).with_context(|| format!("{}:{} занят: --port N", opts.host, opts.port))?;
    let addr = listener.local_addr()?;
    eprintln!("keyfill serve: http://{addr}  WebSocket ws://{addr}/ws  (Ctrl+C — выход)");
    // Строка для скриптов и тестов: порт мог выбрать сам сервер (--port 0).
    println!("{}", json!({ "keyfill": 1, "serve": format!("{addr}"), "channels": channels.keys().collect::<Vec<_>>() }));
    let _ = std::io::stdout().flush();
    let server = Arc::new(Server { channels, hub, token: opts.token, loopback: loopback(&opts.host) });
    for stream in listener.incoming().flatten() {
        let server = server.clone();
        std::thread::spawn(move || {
            let _ = server.connection(stream);
        });
    }
    Ok(())
}

// ─── Поток канала ───────────────────────────────────────────────────────────

/// Куда идёт команда: в эфир или в отложенные (preview).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Target {
    Program,
    Preview,
}

enum Req {
    /// Команды слоям: в эфир (на следующем кадре) или в отложенные.
    Commands { cmds: Vec<Entry>, to: Target, reply: Sender<Value> },
    /// Отложенные — в эфир одной пачкой на следующем кадре.
    Take { reply: Sender<Value> },
    Discard { reply: Sender<Value> },
    State { reply: Sender<Value> },
    Schema { reply: Sender<Value> },
    /// Кадр эфира (или preview) PNG и номер кадра эфира, к которому он относится.
    Monitor { scale: f32, preview: bool, reply: Sender<Result<(Vec<u8>, u64), String>> },
}

struct RunnerContext {
    number: u32,
    file: PathBuf,
    bundle: Bundle,
    run: RunOptions,
    eval: EvalOptions,
    journal_dir: PathBuf,
    resume: bool,
    hub: Hub,
}

/// Отложенные команды и их preview: снимок эфира плюс отложенное, кадр — когда всё улеглось.
#[derive(Default)]
struct Preview {
    staged: Vec<Entry>,
    view: Option<PreviewView>,
}

struct PreviewView {
    t: f64,
    frame: Frame,
}

struct Runner {
    number: u32,
    bundle: Bundle,
    run: RunOptions,
    eval: EvalOptions,
    program: Session,
    preview: Preview,
    journal_path: PathBuf,
    journal: std::fs::File,
    hub: Hub,
    start: Instant,
    /// Номер кадра, который построен последним.
    frame_no: u64,
    last: Option<Frame>,
    /// Команды в эфир, ждущие границы кадра.
    pending: Vec<(Vec<Entry>, Sender<Value>, bool)>,
    events_sent: usize,
    diags_seen: BTreeSet<String>,
    renderer: Renderer,
}

impl Runner {
    fn start(ctx: RunnerContext) -> Result<Runner> {
        let restore = if ctx.resume { resume_batch(&ctx)? } else { None };
        let mut program = Session::open(&ctx.bundle, &ctx.run, &[], &[])?;
        if program.state().is_none() {
            return Err(crate::usage(format!("{}: serve играет каналы (defineChannel), а файл экспортирует сцену", slash(&ctx.file))));
        }
        program.labels = false;
        // Журнал не перезаписывается: два запуска в одну секунду получают `-2`, `-3`.
        let stamp = utc_stamp();
        let (journal_path, mut journal) = (1..)
            .map(|i| {
                let suffix = if i == 1 { String::new() } else { format!("-{i}") };
                ctx.journal_dir.join(format!("ch{}-{stamp}{suffix}{JOURNAL_SUFFIX}", ctx.number))
            })
            .find_map(|p| std::fs::File::create_new(&p).ok().map(|f| (p, f)))
            .context("журнал не создан")?;
        let channel = relative(&ctx.file, &ctx.journal_dir);
        let header = Header { keyfill: 1, channel, seed: ctx.run.seed, now: ctx.run.now_ms };
        writeln!(journal, "{}", serde_json::to_string(&header)?)?;
        let mut runner = Runner {
            number: ctx.number,
            bundle: ctx.bundle,
            run: ctx.run,
            eval: ctx.eval,
            program,
            preview: Preview::default(),
            journal_path,
            journal,
            hub: ctx.hub,
            start: Instant::now(),
            frame_no: 0,
            last: None,
            pending: Vec::new(),
            events_sent: 0,
            diags_seen: BTreeSet::new(),
            renderer: Renderer::default(),
        };
        if let Some(batch) = restore {
            let acks = runner.program.command(batch.cmds)?;
            if let Some(bad) = acks.iter().find(|a| a.status != AckStatus::Ok) {
                eprintln!("keyfill serve: канал {}: при подъёме {} — {}", runner.number, bad.cmd, bad.errors.join("; "));
            }
        }
        runner.tick(0.0)?;
        Ok(runner)
    }

    fn fps(&self) -> f64 {
        self.program.fps
    }

    fn run(&mut self, rx: Receiver<Req>) {
        loop {
            let next = self.start + Duration::from_secs_f64((self.frame_no + 1) as f64 / self.fps());
            let wait = next.saturating_duration_since(Instant::now());
            match rx.recv_timeout(wait) {
                Ok(req) => self.handle(req),
                Err(RecvTimeoutError::Timeout) => {
                    // Кадр по часам: если не успевали, пропускаем кадры, а не догоняем их.
                    let due = (self.start.elapsed().as_secs_f64() * self.fps()).floor() as u64;
                    self.frame_no = due.max(self.frame_no + 1);
                    let t = self.program.frame_time(self.frame_no);
                    if let Err(e) = self.tick(t) {
                        self.hub.send(&json!({ "type": "error", "channel": self.number, "message": format!("{e:#}") }));
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    /// Кадр `t`: команды, пришедшие с прошлого кадра, затем движение, кадр и события.
    fn tick(&mut self, t: f64) -> Result<()> {
        self.program.advance(t)?;
        let pending = std::mem::take(&mut self.pending);
        let mut changed = false;
        for (cmds, reply, take) in pending {
            let acks = self.program.command(cmds)?;
            changed = true;
            let _ = reply.send(json!({ "type": "ack", "channel": self.number, "to": Target::Program, "frame": self.frame_no, "take": take, "results": acks }));
        }
        for b in self.program.take_recorded() {
            writeln!(self.journal, "{}", serde_json::to_string(&b)?)?;
        }
        if changed {
            self.journal.flush()?;
            self.broadcast_state();
            // Эфир изменился: preview — снимок эфира плюс отложенное, его надо пересобрать.
            if !self.preview.staged.is_empty() {
                let staged = self.preview.staged.clone();
                self.preview = Preview { staged: Vec::new(), view: None };
                let _ = self.stage(staged);
            }
        }
        let frame = self.program.frame_at(t, self.eval)?;
        self.broadcast_events();
        self.broadcast_diagnostics(&frame);
        self.last = Some(frame);
        Ok(())
    }

    fn handle(&mut self, req: Req) {
        match req {
            Req::Commands { cmds, to: Target::Program, reply } => self.pending.push((cmds, reply, false)),
            Req::Commands { cmds, to: Target::Preview, reply } => {
                let v = match self.stage(cmds) {
                    Ok(acks) => json!({ "type": "ack", "channel": self.number, "to": Target::Preview, "results": acks }),
                    Err(e) => json!({ "type": "error", "channel": self.number, "message": format!("{e:#}") }),
                };
                let _ = reply.send(v);
            }
            Req::Take { reply } => {
                let staged = std::mem::take(&mut self.preview.staged);
                self.preview.view = None;
                if staged.is_empty() {
                    let _ = reply.send(json!({ "type": "ack", "channel": self.number, "take": true, "results": [] }));
                } else {
                    self.pending.push((staged, reply, true));
                }
                self.broadcast_preview();
            }
            Req::Discard { reply } => {
                self.preview = Preview::default();
                self.broadcast_preview();
                let _ = reply.send(json!({ "type": "ack", "channel": self.number, "discard": true }));
            }
            Req::State { reply } => {
                let _ = reply.send(self.state_json());
            }
            Req::Schema { reply } => {
                let _ = reply.send(self.schema_json());
            }
            Req::Monitor { scale, preview, reply } => {
                let frame = self.frame_no;
                let _ = reply.send(self.monitor(scale, preview).map(|png| (png, frame)).map_err(|e| format!("{e:#}")));
            }
        }
    }

    /// Добавляет команды в отложенные: preview пересобирается, отклонённые не попадают.
    fn stage(&mut self, cmds: Vec<Entry>) -> Result<Vec<Ack>> {
        let restore = self.program.state().map(|s| s.to_batch(0)).unwrap_or(Batch { frame: 0, cmds: Vec::new() });
        let mut session = Session::open(&self.bundle, &self.run, &[], &[restore])?;
        session.labels = false;
        session.advance(0.0)?;
        // Прежние отложенные уже проверены, но эфир мог измениться: проверяются заново.
        let mut all = self.preview.staged.clone();
        let old = all.len();
        all.extend(cmds);
        let acks = session.command(all.clone())?;
        let staged: Vec<Entry> = all.into_iter().zip(&acks).filter(|(_, a)| a.status != AckStatus::Rejected).map(|(e, _)| e).collect();
        let t = session.quiet_time()?;
        let frame = session.frame_at(t, self.eval)?;
        self.preview = Preview { staged, view: Some(PreviewView { t, frame }) };
        self.broadcast_preview();
        Ok(acks.into_iter().skip(old).collect())
    }

    fn state_json(&self) -> Value {
        json!({
            "channel": self.number,
            "frame": self.frame_no,
            "t": self.program.frame_time(self.frame_no),
            "fps": self.fps(),
            "layers": self.program.state().map(|s| &s.layers),
            "preview": self.preview_json(),
            "journal": slash(&self.journal_path),
        })
    }

    fn preview_json(&self) -> Value {
        let staged: Vec<String> = self.preview.staged.iter().map(|e| e.cmd.label()).collect();
        let (t, diagnostics) = match &self.preview.view {
            Some(v) => (Some(v.t), v.frame.diagnostics.0.clone()),
            None => (None, Vec::new()),
        };
        json!({ "staged": staged, "cmds": self.preview.staged, "t": t, "diagnostics": diagnostics })
    }

    fn schema_json(&self) -> Value {
        let scene = &self.program.scene;
        let catalog = scene.channel.clone().unwrap_or_default();
        let components: Vec<_> = scene.components.iter().filter(|c| catalog.contains(&c.name)).collect();
        let stage = scene.stage.as_ref().map(|s| json!({ "size": s.size, "fps": s.fps }));
        json!({ "channel": self.number, "stage": stage, "catalog": catalog, "components": components })
    }

    fn monitor(&mut self, scale: f32, preview: bool) -> Result<Vec<u8>> {
        let frame = if preview {
            match &self.preview.view {
                Some(v) => &v.frame,
                None => self.last.as_ref().context("кадра ещё нет")?,
            }
        } else {
            self.last.as_ref().context("кадра ещё нет")?
        };
        let pixmap = self.renderer.render(frame, &RenderOptions { scale, background: None })?;
        keyfill_render::png(pixmap, Png::Fast)
    }

    fn broadcast_events(&mut self) {
        let log = &self.program.motion.events_log;
        for e in log.iter().skip(self.events_sent) {
            let frame = self.program.frame_index(e.t);
            self.hub.send(&json!({ "type": "event", "channel": self.number, "t": e.t, "frame": frame, "source": e.source, "kind": e.kind, "name": e.name }));
        }
        self.events_sent = log.len();
    }

    /// Новые диагностики эфира (например, применённый `else`): каждая — один раз, пока держится.
    fn broadcast_diagnostics(&mut self, frame: &Frame) {
        let mut now = BTreeSet::new();
        for d in &frame.diagnostics.0 {
            let key = format!("{}|{}|{}", d.rule, d.node, d.message);
            if !self.diags_seen.contains(&key) {
                self.hub.send(&json!({ "type": "diagnostic", "channel": self.number, "frame": self.frame_no, "diagnostic": d }));
            }
            now.insert(key);
        }
        self.diags_seen = now;
    }

    fn broadcast_state(&self) {
        self.hub.send(&json!({ "type": "state", "channel": self.number, "frame": self.frame_no, "layers": self.program.state().map(|s| &s.layers) }));
    }

    fn broadcast_preview(&self) {
        self.hub.send(&json!({ "type": "preview", "channel": self.number, "preview": self.preview_json() }));
    }
}

/// Последний журнал канала → снимок в его конце → команды подъёма сразу «в эфир».
fn resume_batch(ctx: &RunnerContext) -> Result<Option<Batch>> {
    let prefix = format!("ch{}-", ctx.number);
    let mut journals: Vec<PathBuf> = std::fs::read_dir(&ctx.journal_dir)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&prefix) && n.ends_with(JOURNAL_SUFFIX)))
        .collect();
    journals.sort();
    let Some(last) = journals.pop() else {
        eprintln!("keyfill serve: канал {}: журналов нет, начинаю с пустого канала", ctx.number);
        return Ok(None);
    };
    let journal = Journal::read(&last)?;
    let mut run = ctx.run.clone();
    run.seed = journal.header.seed;
    run.now_ms = journal.header.now;
    let mut session = Session::open(&ctx.bundle, &run, &[], &journal.batches)?;
    let end = session.quiet_time()?;
    session.advance(end)?;
    let batch = session.state().map(|s| s.to_batch(0)).filter(|b| !b.cmds.is_empty());
    eprintln!("keyfill serve: канал {}: подъём из {} — {}", ctx.number, slash(&last), batch.as_ref().map_or("пусто".to_string(), |b| format!("{} команд", b.cmds.len())));
    Ok(batch)
}

// ─── Сеть ───────────────────────────────────────────────────────────────────

/// Рассылка клиентам WebSocket: события, состояние, preview.
#[derive(Clone, Default)]
struct Hub(Arc<Mutex<Vec<Sender<String>>>>);

impl Hub {
    fn send(&self, v: &Value) {
        let text = v.to_string();
        let mut clients = self.0.lock().unwrap_or_else(|e| e.into_inner());
        clients.retain(|c| c.send(text.clone()).is_ok());
    }

    fn subscribe(&self) -> Receiver<String> {
        let (tx, rx) = mpsc::channel();
        self.0.lock().unwrap_or_else(|e| e.into_inner()).push(tx);
        rx
    }
}

struct Server {
    channels: BTreeMap<u32, Sender<Req>>,
    hub: Hub,
    token: Option<String>,
    loopback: bool,
}

impl Server {
    fn connection(&self, mut stream: TcpStream) -> Result<()> {
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_nodelay(true)?;
        let req = read_request(&mut stream)?;
        // DNS rebinding: страница чужого сайта не достучится до пульта на loopback.
        if self.loopback && !req.local() {
            return respond(&mut stream, 403, "text/plain; charset=utf-8", b"forbidden host", &[]);
        }
        if !self.authorized(&req) {
            return respond(&mut stream, 401, "text/plain; charset=utf-8", b"token required: Authorization: Bearer <token> or ?token=", &[]);
        }
        let header = |name: &str| req.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
        if req.path == "/ws" {
            let Some(key) = header("sec-websocket-key").filter(|_| header("upgrade").is_some_and(|u| u.eq_ignore_ascii_case("websocket"))) else {
                return respond(&mut stream, 400, "text/plain; charset=utf-8", b"WebSocket upgrade expected", &[]);
            };
            let accept = tungstenite::handshake::derive_accept_key(key.as_bytes());
            let head = format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n");
            stream.write_all(head.as_bytes())?;
            return self.socket(stream);
        }
        let json_ok = |stream: &mut TcpStream, v: &Value| respond(stream, 200, "application/json; charset=utf-8", v.to_string().as_bytes(), &[]);
        match req.path.as_str() {
            "/" => json_ok(&mut stream, &json!({ "keyfill": 1, "version": env!("CARGO_PKG_VERSION"), "channels": self.channels.keys().collect::<Vec<_>>(), "ws": "/ws" })),
            "/schema" => json_ok(&mut stream, &self.all(|reply| Req::Schema { reply })),
            "/state" => json_ok(&mut stream, &self.all(|reply| Req::State { reply })),
            path => {
                // /monitor/1.png?scale=0.25[&preview=1]
                let Some(n) = path.strip_prefix("/monitor/").and_then(|p| p.strip_suffix(".png")).and_then(|n| n.parse::<u32>().ok()) else {
                    return respond(&mut stream, 404, "text/plain; charset=utf-8", b"not found", &[]);
                };
                let Some(ch) = self.channels.get(&n) else {
                    return respond(&mut stream, 404, "text/plain; charset=utf-8", b"no such channel", &[]);
                };
                let scale = req.q("scale").and_then(|v| v.parse::<f32>().ok()).filter(|s| *s > 0.0 && *s <= 2.0).unwrap_or(0.25);
                let preview = req.q("preview").is_some_and(|v| v == "1" || v == "true");
                let (reply, rx) = mpsc::channel();
                ch.send(Req::Monitor { scale, preview, reply })?;
                match rx.recv()? {
                    // Номер кадра: живой кадр можно сверить с воспроизведением журнала.
                    Ok((png, frame)) => respond(&mut stream, 200, "image/png", &png, &[("X-Keyfill-Frame", frame.to_string())]),
                    Err(e) => respond(&mut stream, 500, "text/plain; charset=utf-8", e.as_bytes(), &[]),
                }
            }
        }
    }

    fn authorized(&self, req: &Request) -> bool {
        let Some(token) = &self.token else { return true };
        let bearer = req.headers.iter().find(|(k, _)| k == "authorization").and_then(|(_, v)| v.strip_prefix("Bearer "));
        let given = bearer.or(req.q("token")).unwrap_or("");
        // Сравнение без раннего выхода: время ответа не выдаёт, сколько символов совпало.
        given.len() == token.len() && given.bytes().zip(token.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
    }

    /// Запрос ко всем каналам: `{ "channels": { "1": … } }`.
    fn all(&self, make: impl Fn(Sender<Value>) -> Req) -> Value {
        let mut out = serde_json::Map::new();
        for (n, ch) in &self.channels {
            let (reply, rx) = mpsc::channel();
            if ch.send(make(reply)).is_ok()
                && let Ok(v) = rx.recv_timeout(Duration::from_secs(5))
            {
                out.insert(n.to_string(), v);
            }
        }
        json!({ "keyfill": 1, "channels": out })
    }

    /// Клиент WebSocket: команды в каналы, ответы и рассылка — обратно.
    fn socket(&self, stream: TcpStream) -> Result<()> {
        stream.set_read_timeout(Some(Duration::from_millis(20)))?;
        let mut ws = WebSocket::from_raw_socket(stream, Role::Server, None);
        let events = self.hub.subscribe();
        let hello = json!({ "type": "hello", "keyfill": 1, "version": env!("CARGO_PKG_VERSION"), "channels": self.channels.keys().collect::<Vec<_>>() });
        ws.send(Message::text(hello.to_string()))?;
        let (reply_tx, reply_rx) = mpsc::channel::<Value>();
        loop {
            match ws.read() {
                Ok(Message::Text(text)) => {
                    if let Some(v) = self.message(&text, &reply_tx) {
                        ws.send(Message::text(v.to_string()))?;
                    }
                }
                Ok(Message::Close(_)) => return Ok(()),
                Ok(_) => {}
                Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Err(_) => return Ok(()),
            }
            while let Ok(v) = reply_rx.try_recv() {
                ws.send(Message::text(v.to_string()))?;
            }
            while let Ok(text) = events.try_recv() {
                ws.send(Message::text(text))?;
            }
        }
    }

    /// Сообщение клиента. Ответ сразу — для ошибок разбора; остальное приходит из канала.
    fn message(&self, text: &str, reply: &Sender<Value>) -> Option<Value> {
        let mut v: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(e) => return Some(json!({ "type": "error", "message": format!("не JSON: {e}") })),
        };
        let id = v.get("id").cloned();
        let fail = |message: String| Some(json!({ "type": "error", "id": id, "message": message }));
        let Some(obj) = v.as_object_mut() else { return fail("сообщение — объект { cmd, ... }".into()) };
        let channel = match obj.remove("channel") {
            None => 1,
            Some(Value::Number(n)) => match n.as_u64() {
                Some(n) => n as u32,
                None => return fail("channel — номер канала".into()),
            },
            Some(_) => return fail("channel — номер канала".into()),
        };
        let Some(ch) = self.channels.get(&channel) else {
            return fail(format!("нет канала {channel}; есть: {}", self.channels.keys().map(u32::to_string).collect::<Vec<_>>().join(", ")));
        };
        let to = match obj.remove("to").as_ref().and_then(Value::as_str) {
            None | Some("program") => Target::Program,
            Some("preview") => Target::Preview,
            Some(other) => return fail(format!("to — program или preview, получено {other:?}")),
        };
        let cmd = obj.get("cmd").and_then(Value::as_str).unwrap_or("").to_string();
        // Ответ канала придёт в общий поток ответов клиента; id запроса кладётся в него здесь.
        let (tx, rx) = mpsc::channel::<Value>();
        let req = match cmd.as_str() {
            "take" => Req::Take { reply: tx },
            "discard" => Req::Discard { reply: tx },
            "state" => Req::State { reply: tx },
            "schema" => Req::Schema { reply: tx },
            "batch" => {
                let cmds = match obj.remove("cmds").map(serde_json::from_value::<Vec<Entry>>) {
                    Some(Ok(c)) => c,
                    Some(Err(e)) => return fail(format!("batch: {e}")),
                    None => return fail("batch: нужен cmds — массив команд".into()),
                };
                Req::Commands { cmds, to, reply: tx }
            }
            _ => match serde_json::from_value::<Entry>(Value::Object(obj.clone())) {
                Ok(e) => Req::Commands { cmds: vec![e], to, reply: tx },
                Err(e) => return fail(format!("команда не разобралась: {e}; команды: load, play, next, stop, update, invoke, clear, batch, take, discard, state, schema")),
            },
        };
        if ch.send(req).is_err() {
            return fail(format!("канал {channel} остановлен"));
        }
        let reply = reply.clone();
        std::thread::spawn(move || {
            if let Ok(mut v) = rx.recv() {
                if let Some(o) = v.as_object_mut() {
                    o.insert("id".into(), id.unwrap_or(Value::Null));
                }
                let _ = reply.send(v);
            }
        });
        None
    }
}

// ─── Мелочи ─────────────────────────────────────────────────────────────────

/// `target` относительно каталога `base`: в заголовке журнала канал записан от каталога журнала.
fn relative(target: &Path, base: &Path) -> String {
    let (Ok(t), Ok(b)) = (std::path::absolute(target), std::path::absolute(base)) else { return slash(target) };
    let tc: Vec<_> = t.components().collect();
    let bc: Vec<_> = b.components().collect();
    let common = tc.iter().zip(&bc).take_while(|(a, b)| a == b).count();
    if common == 0 {
        return slash(&t);
    }
    let mut out = PathBuf::new();
    for _ in common..bc.len() {
        out.push("..");
    }
    for c in &tc[common..] {
        out.push(c);
    }
    slash(&out)
}

/// Время UTC для имени журнала: `20261016-153012`; имена сортируются по времени.
fn utc_stamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    // Дни от 1970-01-01 → дата (алгоритм Хиннанта, civil_from_days).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_for_journal_header() {
        let base = std::env::current_dir().unwrap();
        assert_eq!(relative(&base.join("examples/channel/channel.tsx"), &base.join(".keyfill/journal")), "../../examples/channel/channel.tsx");
        assert_eq!(relative(&base.join("a/channel.tsx"), &base.join("a")), "channel.tsx");
    }

    #[test]
    fn stamp_looks_like_utc_time() {
        let s = utc_stamp();
        assert_eq!(s.len(), 15, "{s}");
        assert!(s.starts_with("20"), "{s}");
        assert_eq!(&s[8..9], "-");
    }
}
