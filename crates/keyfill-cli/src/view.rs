//! `keyfill view`: локальный веб-просмотрщик. Галерея всех выводов из каталога и живая сцена:
//! кадр в любой момент, дамп, события, inspect по клику на узел. Исходники сцены
//! отслеживаются: после правки страница перерисовывается сама.
//!
//! Сервер намеренно простой: std::net, запросы обрабатываются по одному, слушает только
//! loopback — и IPv4, и IPv6: браузер на Windows ходит на `localhost` сначала через `::1`,
//! и без IPv6-слушателя каждый запрос ждал бы отказа ~200 мс.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context as _, Result};
use keyfill_core::look::Rgba;
use keyfill_core::{EvalOptions, Frame, Scene};
use keyfill_js::RunOptions;
use keyfill_render::RenderOptions;
use serde_json::{Value, json};

use crate::Session;
use crate::session::CueArg;

const INDEX: &str = include_str!("view/index.html");

pub struct ViewOptions {
    pub entry: Option<PathBuf>,
    pub dir: PathBuf,
    pub port: u16,
    pub run: RunOptions,
    pub extra: Vec<CueArg>,
    pub eval: EvalOptions,
}

struct SceneState {
    entry: PathBuf,
    fingerprint: u64,
    source: Result<crate::Bundle, String>,
    /// Пачки команд, если сцена — журнал канала, и его сид и часы.
    batches: Vec<crate::channel::Batch>,
    clock: Option<(u32, f64)>,
    /// Входные файлы последней удачной сборки: по ним видно правки, в том числе вне каталога сцены.
    inputs: Vec<PathBuf>,
    session: Option<Session>,
    info: Option<Value>,
}

struct State {
    opts: ViewOptions,
    scene: Option<SceneState>,
    /// Буфер кадра и кэш глифов живут между запросами кадров.
    renderer: keyfill_render::Renderer,
}

pub fn serve(opts: ViewOptions) -> Result<()> {
    let v4 = TcpListener::bind(("127.0.0.1", opts.port)).with_context(|| format!("порт {} занят: --port N", opts.port))?;
    // IPv6 может быть выключен в системе — тогда хватит IPv4.
    let v6 = TcpListener::bind(("::1", opts.port)).ok();
    std::fs::create_dir_all(&opts.dir).ok();
    eprintln!("keyfill view: http://localhost:{}  (каталог {}, Ctrl+C — выход)", opts.port, opts.dir.display());
    let scene = opts.entry.clone().map(|entry| SceneState { entry, fingerprint: 0, source: Err(String::new()), batches: Vec::new(), clock: None, inputs: Vec::new(), session: None, info: None });
    let mut state = State { opts, scene, renderer: Default::default() };
    refresh(&mut state);

    // Слушатели принимают соединения в своих потоках, запрос читается в потоке соединения
    // с таймаутом (браузер открывает сокеты заранее и может молчать), а обработка одна:
    // состояние сцены не делится.
    let (tx, rx) = std::sync::mpsc::channel::<(TcpStream, Request)>();
    for listener in std::iter::once(v4).chain(v6) {
        let tx = tx.clone();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let tx = tx.clone();
                std::thread::spawn(move || {
                    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                    let _ = stream.set_nodelay(true);
                    if let Ok(req) = read_request(&mut stream) {
                        let _ = tx.send((stream, req));
                    }
                });
            }
        });
    }
    drop(tx);
    for (mut stream, req) in rx {
        if let Err(e) = handle(&mut state, &mut stream, req) {
            let _ = respond(&mut stream, 500, "text/plain; charset=utf-8", format!("{e:#}").as_bytes(), &[]);
        }
    }
    Ok(())
}

// ─── HTTP ───────────────────────────────────────────────────────────────────

struct Request {
    path: String,
    query: Vec<(String, String)>,
    range: Option<String>,
    host: Option<String>,
}

impl Request {
    /// Запрос пришёл на loopback-имя: страница с чужого сайта, переименованного DNS в
    /// 127.0.0.1 (DNS rebinding), не прочтёт файлы просмотрщика.
    fn local(&self) -> bool {
        let Some(host) = &self.host else { return true };
        let name = match host.strip_prefix('[') {
            Some(v6) => v6.split(']').next().unwrap_or(""),
            None => host.split(':').next().unwrap_or(""),
        };
        matches!(name, "localhost" | "127.0.0.1" | "::1")
    }
}

impl Request {
    fn q(&self, k: &str) -> Option<&str> {
        self.query.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str())
    }

    fn num(&self, k: &str) -> Option<f64> {
        self.q(k).and_then(|v| v.parse().ok())
    }
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(b) => {
                        out.push(b);
                        i += 3;
                        continue;
                    }
                    None => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn read_request(stream: &mut TcpStream) -> Result<Request> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > 64 * 1024 {
            anyhow::bail!("слишком длинный запрос");
        }
    }
    let head = String::from_utf8_lossy(&buf);
    let mut lines = head.lines();
    let target = lines.next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("/").to_string();
    let headers: Vec<(String, String)> =
        lines.filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))).collect();
    let header = |name: &str| headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
    let range = header("range");
    let host = header("host");
    let (path, qs) = target.split_once('?').unwrap_or((&target, ""));
    let query = qs.split('&').filter(|p| !p.is_empty()).map(|p| {
        let (k, v) = p.split_once('=').unwrap_or((p, ""));
        (decode(k), decode(v))
    });
    Ok(Request { path: decode(path), query: query.collect(), range, host })
}

fn respond(stream: &mut TcpStream, status: u16, ctype: &str, body: &[u8], extra: &[(&str, String)]) -> Result<()> {
    let reason = match status {
        200 => "OK",
        206 => "Partial Content",
        403 => "Forbidden",
        404 => "Not Found",
        416 => "Range Not Satisfiable",
        _ => "Error",
    };
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n",
        body.len()
    );
    for (k, v) in extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    // Заголовок и тело одной записью: две мелкие записи подряд ловят задержку Nagle.
    let mut out = head.into_bytes();
    out.extend_from_slice(body);
    stream.write_all(&out)?;
    Ok(())
}

fn json_response(stream: &mut TcpStream, v: &Value) -> Result<()> {
    respond(stream, 200, "application/json; charset=utf-8", v.to_string().as_bytes(), &[])
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("webm") => "video/webm",
        Some("mp4") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("json") => "application/json; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        _ => "text/plain; charset=utf-8",
    }
}

fn kind_of(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("png" | "jpg" | "jpeg" | "gif" | "svg") => "image",
        Some("webm" | "mp4") => "video",
        Some("mov") => "mov",
        Some("html") => "html",
        Some("txt" | "md" | "json" | "log" | "csv" | "tsx" | "ts") => "text",
        _ => "file",
    }
}

fn handle(state: &mut State, stream: &mut TcpStream, req: Request) -> Result<()> {
    if !req.local() {
        return respond(stream, 403, "text/plain; charset=utf-8", b"keyfill view: only localhost", &[]);
    }
    match req.path.as_str() {
        "/" | "/index.html" => respond(stream, 200, "text/html; charset=utf-8", INDEX.as_bytes(), &[]),
        "/api/files" => json_response(stream, &files(&state.opts.dir)),
        "/api/scene" => {
            let v = scene_info(state);
            json_response(stream, &v)
        }
        "/api/frame.png" => {
            let t = req.num("t").unwrap_or(0.0);
            let scale = req.num("s").unwrap_or(0.5) as f32;
            let bg = req.q("bg").and_then(Rgba::from_hex);
            let eval = EvalOptions { background: bg.or(state.opts.eval.background), ..state.opts.eval };
            let (_, frame) = frame_at(state, t, eval)?;
            let pixmap = state.renderer.render(&frame, &RenderOptions { scale, background: bg })?;
            let png = keyfill_render::png(pixmap, keyfill_render::Png::Fast)?;
            respond(stream, 200, "image/png", &png, &[])
        }
        "/api/dump" => {
            // Строки дампа с адресами узлов: страница делает строки узлов кликабельными.
            let t = req.num("t").unwrap_or(0.0);
            let eval = state.opts.eval;
            let (session, frame) = frame_at(state, t, eval)?;
            let lines = keyfill_core::dump::lines(&keyfill_core::dump::build(&session.scene, &frame));
            json_response(stream, &json!({ "lines": lines }))
        }
        "/api/inspect" => {
            let t = req.num("t").unwrap_or(0.0);
            let path = req.q("path").unwrap_or("").to_string();
            let eval = state.opts.eval;
            let (session, frame) = frame_at(state, t, eval)?;
            let text = keyfill_core::dump::inspect(&session.scene, &frame, &path).unwrap_or_else(|| format!("узел {path} не найден"));
            respond(stream, 200, "text/plain; charset=utf-8", text.as_bytes(), &[])
        }
        "/api/layout" => {
            // Рамки всех узлов кадра в порядке отрисовки: браузер сам находит узел под курсором.
            let t = req.num("t").unwrap_or(0.0);
            let eval = state.opts.eval;
            let (session, frame) = frame_at(state, t, eval)?;
            let scene = &session.scene;
            let boxes: Vec<Value> = keyfill_core::display::node_boxes(&frame)
                .iter()
                .map(|b| {
                    let node = scene.node(b.id);
                    json!({
                        "path": scene.path(b.id),
                        "kind": node.kind.name(),
                        "x": b.rect.x, "y": b.rect.y, "w": b.rect.w, "h": b.rect.h,
                        "depth": b.depth,
                        "opacity": b.opacity,
                    })
                })
                .collect();
            json_response(stream, &json!({ "size": frame.size, "boxes": boxes }))
        }
        p if p.starts_with("/files/") => serve_file(stream, &state.opts.dir, &p["/files/".len()..], req.range.as_deref()),
        _ => respond(stream, 404, "text/plain; charset=utf-8", b"not found", &[]),
    }
}

// ─── Файлы ──────────────────────────────────────────────────────────────────

fn files(dir: &Path) -> Value {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let Ok(meta) = e.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            let path = e.path();
            let mtime = meta.modified().ok().and_then(|m| m.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as u64);
            out.push(json!({
                "name": e.file_name().to_string_lossy(),
                "size": meta.len(),
                "mtime": mtime,
                "kind": kind_of(&path),
            }));
        }
    }
    out.sort_by(|a, b| b["mtime"].as_u64().cmp(&a["mtime"].as_u64()));
    Value::Array(out)
}

/// Только имя файла внутри каталога: ни подкаталогов, ни `..`, ни путей вида `C:файл`,
/// которые на Windows при `join` заменяют весь каталог.
fn plain_name(name: &str) -> bool {
    let mut parts = Path::new(name).components();
    matches!((parts.next(), parts.next()), (Some(std::path::Component::Normal(_)), None)) && !name.contains(['/', '\\', ':'])
}

fn serve_file(stream: &mut TcpStream, dir: &Path, name: &str, range: Option<&str>) -> Result<()> {
    if !plain_name(name) {
        return respond(stream, 404, "text/plain; charset=utf-8", b"not found", &[]);
    }
    let path = dir.join(name);
    let Ok(data) = std::fs::read(&path) else {
        return respond(stream, 404, "text/plain; charset=utf-8", b"not found", &[]);
    };
    let ctype = content_type(&path);
    let accept = ("Accept-Ranges", "bytes".to_string());
    // Range нужен браузеру, чтобы перематывать видео.
    if let Some(spec) = range.and_then(|r| r.strip_prefix("bytes=")) {
        let (a, b) = spec.split_once('-').unwrap_or((spec, ""));
        let len = data.len();
        let last = len.saturating_sub(1);
        let (start, end) = match (a.parse::<usize>(), b.parse::<usize>()) {
            // `bytes=-N` — последние N байт.
            (Err(_), Ok(n)) => (len.saturating_sub(n), last),
            (Ok(s), Ok(e)) => (s, e.min(last)),
            (Ok(s), Err(_)) => (s, last),
            (Err(_), Err(_)) => (0, last),
        };
        if start >= len || end < start {
            return respond(stream, 416, ctype, b"", &[("Content-Range", format!("bytes */{len}"))]);
        }
        return respond(stream, 206, ctype, &data[start..=end], &[accept, ("Content-Range", format!("bytes {start}-{end}/{len}"))]);
    }
    respond(stream, 200, ctype, &data, &[accept])
}

// ─── Сцена ──────────────────────────────────────────────────────────────────

/// Отпечаток исходников: размеры и времена изменения входных файлов последней сборки.
/// Пока сборка ни разу не удалась, входы неизвестны — тогда смотрим каталог сцены.
fn fingerprint(entry: &Path, inputs: &[PathBuf]) -> u64 {
    let mut h = DefaultHasher::new();
    if !inputs.is_empty() {
        for path in std::iter::once(entry).chain(inputs.iter().map(PathBuf::as_path)) {
            path.hash(&mut h);
            if let Ok(meta) = std::fs::metadata(path) {
                meta.len().hash(&mut h);
                meta.modified().ok().and_then(|m| m.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_nanos()).hash(&mut h);
            }
        }
        return h.finish();
    }
    let root = entry.parent().unwrap_or(Path::new("."));
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.path());
        for e in entries {
            let path = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if matches!(name.as_str(), "node_modules" | "target" | "out" | ".git" | ".keyfill") {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(meta) = e.metadata() {
                path.hash(&mut h);
                meta.len().hash(&mut h);
                meta.modified().ok().and_then(|m| m.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_nanos()).hash(&mut h);
            }
        }
    }
    h.finish()
}

/// Пересобирает сцену, если изменились её исходники. Вызывается из `/api/scene`: страница
/// опрашивает его раз в секунду, а запросы кадров не тратят время на проверку файлов.
fn refresh(state: &mut State) {
    let Some(sc) = &mut state.scene else { return };
    let fp = fingerprint(&sc.entry, &sc.inputs);
    if fp != sc.fingerprint {
        // Журнал канала: пачки из файла, бандл — его канал (часы и сид журнала — из заголовка).
        sc.source = if crate::channel::is_journal(&sc.entry) {
            crate::channel::Journal::read(&sc.entry)
                .and_then(|j| {
                    let bundle = crate::bundle(&j.channel_path(&sc.entry))?;
                    sc.batches = j.batches;
                    sc.clock = Some((j.header.seed, j.header.now));
                    Ok(bundle)
                })
                .map_err(|e| format!("{e:#}"))
        } else {
            crate::bundle(&sc.entry).map_err(|e| format!("{e:#}"))
        };
        if let Ok(b) = &sc.source {
            sc.inputs = b.inputs.clone();
        }
        sc.fingerprint = fingerprint(&sc.entry, &sc.inputs);
        sc.session = None;
        sc.info = None;
    }
}

fn open(state: &State) -> Result<Session> {
    let sc = state.scene.as_ref().context("keyfill view запущен без сцены")?;
    let source = sc.source.as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut run = state.opts.run.clone();
    if let Some((seed, now)) = sc.clock {
        run.seed = seed;
        run.now_ms = now;
    }
    Session::open(source, &run, &state.opts.extra, &sc.batches)
}

/// Кадр в момент `t` на живой сессии: она перематывается сама, если время идёт назад.
/// После ошибки сессия выбрасывается: её состояние после упавшей команды не равно сценарию.
fn frame_at(state: &mut State, t: f64, eval: EvalOptions) -> Result<(&Session, Frame)> {
    if state.scene.as_ref().is_some_and(|s| s.session.is_none()) {
        let session = open(state)?;
        state.scene.as_mut().expect("scene").session = Some(session);
    }
    let sc = state.scene.as_mut().context("keyfill view запущен без сцены")?;
    let session = sc.session.as_mut().expect("session");
    let t = session.nearest_frame(t);
    match session.frame_at(t, eval) {
        Ok(frame) => Ok((sc.session.as_ref().expect("session"), frame)),
        Err(e) => {
            sc.session = None;
            Err(e)
        }
    }
}

fn scene_info(state: &mut State) -> Value {
    refresh(state);
    let Some(sc) = &state.scene else { return json!({ "scene": null }) };
    if let Some(info) = &sc.info {
        return info.clone();
    }
    let fingerprint = sc.fingerprint;
    let entry = sc.entry.display().to_string();
    let info = match probe(state) {
        Ok(mut v) => {
            v["entry"] = json!(entry);
            v["version"] = json!(fingerprint.to_string());
            v
        }
        Err(e) => json!({ "entry": entry, "version": fingerprint.to_string(), "error": format!("{e:#}") }),
    };
    if let Some(sc) = &mut state.scene {
        sc.info = Some(info.clone());
    }
    info
}

fn probe(state: &State) -> Result<Value> {
    let mut s = open(state)?;
    let settled = s.settled_time()?;
    let end = s.end_time()?;
    s.advance(end)?;
    let mut markers: Vec<Value> = Vec::new();
    for e in s.motion.events_log.iter().filter(|e| e.kind == "marker") {
        if !markers.iter().any(|m| m["name"] == e.name.as_str()) {
            markers.push(json!({ "name": e.name, "t": e.t }));
        }
    }
    let events: Vec<Value> = s.motion.events_log.iter().map(|e| json!({ "t": e.t, "label": e.label() })).collect();
    let cues: Vec<Value> = s.cues().iter().map(|c| json!({ "t": c.t, "cmd": c.cmd.name(), "label": c.label() })).collect();
    let size = s.scene.stage.as_ref().map_or([1920.0, 1080.0], |st| st.size);
    Ok(json!({
        "fps": s.fps,
        "size": size,
        "end": end,
        "settled": settled,
        "markers": markers,
        "events": events,
        "cues": cues,
        "nodes": node_paths(&s.scene),
    }))
}

fn node_paths(scene: &Scene) -> Vec<String> {
    scene.nodes.values().filter(|n| n.kind != keyfill_core::scene::Kind::Stage).map(|n| scene.path(n.id)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_file_names() {
        assert!(plain_name("strip.png"));
        for bad in ["../Cargo.toml", "C:Cargo.toml", "C:\\x", "a/b", "..", "", "/etc"] {
            assert!(!plain_name(bad), "{bad}");
        }
    }

    #[test]
    fn url_decoding() {
        assert_eq!(decode("Scoreboard%231%2Fhome"), "Scoreboard#1/home");
        assert_eq!(decode("%D0%90+b"), "А b");
        assert_eq!(decode("100%"), "100%");
    }
}
