use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use kine_cli::time::{self, Moment, Offset, TimeSpec};
use kine_cli::{Failure, Session, filmstrip};
use kine_core::diag::Level;
use kine_core::look::parse_color;
use kine_core::{EvalOptions, Mode};
use kine_js::{JsException, RunOptions};
use kine_render::RenderOptions;

const USAGE: &str = "\
kine — рендер TSX-сцен без браузера

  kine render <entry> [-o out.png] [--scale N] [--dump]
  kine dump <entry>                   текстовое дерево сцены с геометрией, движением и диагностиками
  kine inspect <entry> <path>         один узел: значения, их происхождение, движение, исходник
  kine check <entry> [--json]         только диагностики
  kine filmstrip <entry> [--t a..b] [-o strip.png] [--n 8] [--cols 4] [--tile 480] [--dump]
                                      раскадровка: n моментов диапазона одной картинкой
  kine video <entry> [--t a..b] [-o out.mov] [--codec prores|qtrle|vp9|h264|png] [--scale N]
                                      видео через ffmpeg: .mov — ProRes 4444 с альфой, .webm — VP9 с альфой,
                                      .mp4 — H.264 на подложке, каталог — PNG-последовательность
  kine view [entry] [--dir out] [--port 4747]
                                      веб-просмотрщик: галерея выводов из каталога и живая сцена
                                      с ползунком времени, дампом и inspect по клику; следит за правками

<entry> — сцена (defineScene) или сценарий *.cues.ts (cues). Без сценария сцена получает play в 0.

Время (одна грамматика для --t и --cue):
  1.2  60f         секунды или кадры
  settled          кадр «в эфире»: закончились все in первого play (по умолчанию)
  settled+0.5      смещение
  out  out+0.2     маркер таймлайна: момент, когда воспроизведение впервые его проходит
  0..4             диапазон, для filmstrip и video; по умолчанию от 0 до конца сценария
  --cue \"0 play; 125f stop\"   команды сценария в короткой форме (дополняют *.cues.ts)

Общие флаги:
  --mode dev|air   dev (по умолчанию): нарушение must — ошибка; air: применяются шаги else
  --bg \"#RRGGBB\"   подложка: рисуется под сценой и учитывается в проверке контраста
  --seed N         сид Math.random
  --now MS         значение Date.now

Коды выхода: 0 успех, 1 ошибки в диагностиках, 2 неверный вызов, 3 сборка, 4 исключение JS, 5 сбой kine.";

struct Args {
    rest: Vec<String>,
    flags: Vec<(String, Option<String>)>,
}

const VALUED: &[&str] = &[
    "-o", "--scale", "--bg", "--seed", "--now", "--mode", "--t", "--cue", "--n", "--cols", "--tile", "--codec", "--dir", "--port",
];
const SWITCHES: &[&str] = &["--dump", "--json", "-h", "--help"];

impl Args {
    fn parse(raw: Vec<String>) -> Result<Self> {
        let mut rest = Vec::new();
        let mut flags = Vec::new();
        let mut it = raw.into_iter();
        while let Some(a) = it.next() {
            if VALUED.contains(&a.as_str()) {
                let v = it.next().with_context(|| format!("{a}: нет значения"))?;
                flags.push((a, Some(v)));
            } else if SWITCHES.contains(&a.as_str()) {
                flags.push((a, None));
            } else if a.starts_with("--") {
                bail!("неизвестный флаг {a}");
            } else {
                rest.push(a);
            }
        }
        Ok(Self { rest, flags })
    }

    fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|(k, _)| k == name)
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.flags.iter().find(|(k, _)| k == name).and_then(|(_, v)| v.as_deref())
    }

    fn number<T: std::str::FromStr>(&self, name: &str) -> Result<Option<T>> {
        self.value(name).map(|s| s.parse::<T>().map_err(|_| anyhow::anyhow!("{name}: ожидается число, получено {s:?}"))).transpose()
    }

    fn entry(&self) -> Result<PathBuf> {
        self.rest.get(1).map(PathBuf::from).context("не указан файл сцены")
    }
}

fn run_options(args: &Args) -> Result<RunOptions> {
    let mut opts = RunOptions::default();
    if let Some(s) = args.number("--seed")? {
        opts.seed = s;
    }
    if let Some(n) = args.number("--now")? {
        opts.now_ms = n;
    }
    Ok(opts)
}

fn parse_hex(s: &str) -> Result<kine_core::look::Rgba> {
    let h = s.trim_start_matches('#');
    if h.len() != 6 {
        bail!("--bg: ожидается #RRGGBB");
    }
    let n = |i: usize| u8::from_str_radix(&h[i..i + 2], 16);
    let json = serde_json::json!({ "r": n(0)?, "g": n(2)?, "b": n(4)? });
    parse_color(&json).map_err(anyhow::Error::msg)
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            let code = if e.downcast_ref::<JsException>().is_some() {
                4
            } else {
                match e.downcast_ref::<Failure>() {
                    Some(Failure::Usage) => 2,
                    Some(Failure::Build) => 3,
                    None => 5,
                }
            };
            ExitCode::from(code)
        }
    }
}

/// Всё, что задаёт вызов: флаги, вход и время. Ошибки здесь — неверный вызов (код 2).
struct Call {
    cmd: String,
    entry: PathBuf,
    eval: EvalOptions,
    opts: RunOptions,
    extra: Vec<kine_cli::session::CueArg>,
    time: Option<TimeSpec>,
}

fn call(args: &Args) -> Result<Option<Call>> {
    let Some(cmd) = args.rest.first().map(String::as_str) else { return Ok(None) };
    if matches!(cmd, "help") || args.flag("-h") || args.flag("--help") {
        return Ok(None);
    }
    if !matches!(cmd, "render" | "dump" | "inspect" | "check" | "filmstrip" | "video") {
        bail!("неизвестная команда {cmd}\n\n{USAGE}");
    }
    let background = args.value("--bg").map(parse_hex).transpose()?;
    let mode = match args.value("--mode") {
        None | Some("dev") => Mode::Dev,
        Some("air") => Mode::Air,
        Some(other) => bail!("--mode: ожидается dev или air, получено {other}"),
    };
    let time = args.value("--t").map(time::parse).transpose().context("--t")?;
    match (&time, cmd) {
        (Some(TimeSpec::Range(..)), c) if !matches!(c, "filmstrip" | "video") => {
            bail!("--t: диапазон {} принимают filmstrip и video; здесь нужен момент", args.value("--t").unwrap_or(""))
        }
        (Some(TimeSpec::Moment(_)), c @ ("filmstrip" | "video")) => bail!("--t: {c} принимает диапазон, например 0..2 или settled..settled+1"),
        _ => {}
    }
    Ok(Some(Call {
        cmd: cmd.to_string(),
        entry: args.entry()?,
        eval: EvalOptions { mode, background },
        opts: run_options(args)?,
        extra: args.value("--cue").map(kine_cli::session::parse_cues).transpose()?.unwrap_or_default(),
        time,
    }))
}

fn run() -> Result<ExitCode> {
    let args = Args::parse(std::env::args().skip(1).collect()).context(Failure::Usage)?;
    if args.rest.first().map(String::as_str) == Some("view") {
        return view_cmd(&args);
    }
    let Some(call) = call(&args).context(Failure::Usage)? else {
        println!("{USAGE}");
        return Ok(ExitCode::SUCCESS);
    };
    let source = kine_cli::bundle(&call.entry)?;
    let mut session = Session::open(&source, &call.opts, &call.extra)?;

    if call.cmd == "filmstrip" {
        return filmstrip_cmd(&args, &call, session, &source);
    }
    if call.cmd == "video" {
        return video_cmd(&args, &call, session, &source);
    }

    // Момент разрешается на пробной сессии: settled и маркеры продвигают её время вперёд.
    let moment = match &call.time {
        Some(TimeSpec::Moment(m)) => m.clone(),
        _ => Moment::Settled(Offset::Seconds(0.0)),
    };
    let t = time::resolve(&moment, &mut session).context(Failure::Usage)?;
    let mut session = Session::open(&source, &call.opts, &call.extra)?;
    let frame = session.frame_at(t, call.eval)?;
    for log in &session.logs {
        eprintln!("js {}: {}", log.level, log.message);
    }
    let scene = &session.scene;
    let errors = frame.diagnostics.0.iter().any(|d| d.level == Level::Error);

    match call.cmd.as_str() {
        "render" => {
            let out = args.value("-o").map(PathBuf::from).unwrap_or_else(|| call.entry.with_extension("png"));
            let opts = RenderOptions { scale: args.number("--scale").context(Failure::Usage)?.unwrap_or(1.0), background: call.eval.background };
            let png = kine_render::render_png(&frame, &opts)?;
            std::fs::write(&out, png).with_context(|| format!("не удалось записать {}", out.display()))?;
            if args.flag("--dump") {
                print!("{}", kine_core::dump::dump(scene, &frame));
            }
            eprintln!("written {}", out.display());
        }
        "dump" => print!("{}", kine_core::dump::dump(scene, &frame)),
        "inspect" => {
            let path = args.rest.get(2).context("не указан адрес узла, например LowerThird#1/title").context(Failure::Usage)?;
            match kine_core::dump::inspect(scene, &frame, path) {
                Some(text) => print!("{text}"),
                None => return Err(anyhow::anyhow!("узел {path} не найден; адреса узлов есть в `kine dump`").context(Failure::Usage)),
            }
        }
        "check" => {
            let diags = &frame.diagnostics.0;
            if args.flag("--json") {
                println!("{}", serde_json::to_string_pretty(diags)?);
            } else {
                for d in diags {
                    println!("{d}");
                }
            }
        }
        _ => unreachable!(),
    }
    Ok(if errors { ExitCode::from(1) } else { ExitCode::SUCCESS })
}

fn view_cmd(args: &Args) -> Result<ExitCode> {
    let background = args.value("--bg").map(parse_hex).transpose().context(Failure::Usage)?;
    let mode = match args.value("--mode") {
        None | Some("dev") => Mode::Dev,
        Some("air") => Mode::Air,
        Some(other) => return Err(anyhow::anyhow!("--mode: ожидается dev или air, получено {other}").context(Failure::Usage)),
    };
    let opts = kine_cli::view::ViewOptions {
        entry: args.rest.get(1).map(PathBuf::from),
        dir: PathBuf::from(args.value("--dir").unwrap_or("out")),
        port: args.number("--port").context(Failure::Usage)?.unwrap_or(4747),
        run: run_options(args).context(Failure::Usage)?,
        extra: args.value("--cue").map(kine_cli::session::parse_cues).transpose().context(Failure::Usage)?.unwrap_or_default(),
        eval: EvalOptions { mode, background },
    };
    if let Some(entry) = &opts.entry
        && !entry.exists() {
            return Err(anyhow::anyhow!("сцена {} не найдена", entry.display()).context(Failure::Usage));
        }
    kine_cli::view::serve(opts)?;
    Ok(ExitCode::SUCCESS)
}

/// Диапазон из `--t a..b` или весь сценарий. Пробная сессия разрешает границы (settled,
/// маркеры, конец сценария); кадры рендерит новая сессия с начала.
fn range(call: &Call, probe: &mut Session) -> Result<(f64, f64)> {
    let (from, to) = match &call.time {
        Some(TimeSpec::Range(a, b)) => (time::resolve(a, probe).context(Failure::Usage)?, time::resolve(b, probe).context(Failure::Usage)?),
        _ => (0.0, probe.end_time()?),
    };
    if to < from {
        return Err(anyhow::anyhow!("--t: конец диапазона {to} раньше начала {from}").context(Failure::Usage));
    }
    Ok((from, to))
}

fn video_cmd(args: &Args, call: &Call, mut probe: Session, source: &str) -> Result<ExitCode> {
    let (from, to) = range(call, &mut probe)?;
    let out = args.value("-o").map(PathBuf::from).unwrap_or_else(|| call.entry.with_extension("mov"));
    let codec = match args.value("--codec") {
        Some(c) => kine_cli::video::Codec::parse(c),
        None => kine_cli::video::Codec::for_path(&out),
    }
    .context(Failure::Usage)?;
    let opts = kine_cli::video::VideoOptions {
        from,
        to,
        codec,
        scale: args.number("--scale").context(Failure::Usage)?.unwrap_or(1.0),
        background: call.eval.background,
        out: out.clone(),
    };
    let mut session = Session::open(source, &call.opts, &call.extra)?;
    let report = kine_cli::video::render(&mut session, call.eval, &opts)?;
    for log in &session.logs {
        eprintln!("js {}: {}", log.level, log.message);
    }
    eprintln!("written {} ({} кадров, {from:.2}..{to:.2} с, {})", out.display(), report.frames, codec.name());
    if report.errors > 0 {
        eprintln!("в кадрах {} ошибок диагностики: посмотри kine check --t <момент>", report.errors);
    }
    Ok(if report.errors > 0 { ExitCode::from(1) } else { ExitCode::SUCCESS })
}

fn filmstrip_cmd(args: &Args, call: &Call, mut probe: Session, source: &str) -> Result<ExitCode> {
    let (from, to) = range(call, &mut probe)?;
    let mut session = Session::open(source, &call.opts, &call.extra)?;
    let usage = |r: Result<Option<usize>>| r.context(Failure::Usage);
    let fopts = filmstrip::Options {
        from,
        to,
        count: usage(args.number("--n"))?.unwrap_or(8),
        cols: usage(args.number("--cols"))?.unwrap_or(4),
        tile: args.number("--tile").context(Failure::Usage)?.unwrap_or(480.0),
        background: call.eval.background,
    };
    let times = filmstrip::times(&session, &fopts);
    let strip = filmstrip::build(&mut session, &times, call.eval, &fopts)?;
    for log in &session.logs {
        eprintln!("js {}: {}", log.level, log.message);
    }
    let out = args.value("-o").map(PathBuf::from).unwrap_or_else(|| strip_path(&call.entry));
    let png = kine_render::execute_png(&strip.display, &RenderOptions { scale: 1.0, background: None })?;
    std::fs::write(&out, png).with_context(|| format!("не удалось записать {}", out.display()))?;
    if args.flag("--dump") {
        println!("{}", strip.dumps.join("\n"));
    }
    let span = format!("{}..{} с", times.first().unwrap_or(&0.0), times.last().unwrap_or(&0.0));
    eprintln!("written {} ({} кадров, {span})", out.display(), strip.frames.len());
    let errors = strip.frames.iter().flat_map(|f| &f.diagnostics.0).any(|d| d.level == Level::Error);
    Ok(if errors { ExitCode::from(1) } else { ExitCode::SUCCESS })
}

fn strip_path(entry: &Path) -> PathBuf {
    entry.with_extension("strip.png")
}
