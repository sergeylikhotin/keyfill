use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use kine_cli::ops::{self, At, Job};
use kine_cli::output::{self, Envelope, Log, quote};
use kine_cli::session::{CueArg, parse_cues};
use kine_cli::time::{self, TimeSpec};
use kine_cli::Failure;
use kine_core::diag::{Diagnostic, Level};
use kine_core::dump;
use kine_core::look::num;
use kine_core::{EvalOptions, Mode};
use kine_js::RunOptions;
use serde::Serialize;

const USAGE: &str = "\
kine — рендер TSX-сцен без браузера

Увидеть:
  kine render <entry> [-o out.png] [--scale N] [--dump]
  kine dump <entry> [--path <узел>] [--depth N]
                                      дерево сцены: геометрия, движение, диагностики
  kine inspect <entry> <узел>         один узел: значения, их происхождение, движение, исходник
  kine filmstrip <entry> [--t a..b] [-o strip.png] [--n 8] [--cols 4] [--tile 480] [--dump]
                                      раскадровка: n моментов диапазона одной картинкой
  kine video <entry> [--t a..b] [-o out.mov] [--codec prores|qtrle|vp9|h264|png] [--scale N]
                                      видео через ffmpeg: .mov — ProRes 4444 с альфой, .webm — VP9
                                      с альфой, .mp4 — H.264 на подложке, каталог — PNG-кадры
  kine view [entry] [--dir out] [--port 4747]
                                      веб-просмотрщик: галерея выводов и живая сцена
Проверить:
  kine check <entry> [--t a..b]       нарушения must и правил на каждом кадре диапазона
                                      (по умолчанию — весь сценарий)
Понять:
  kine schema <entry>                 пропсы сцены, компоненты, части, команды, маркеры, сценарий

<entry> — сцена (defineScene) или сценарий *.cues.ts (cues). Без сценария сцена получает play в 0.

Время (одна грамматика для --t и --cue):
  1.2  60f         секунды или кадры
  settled          кадр «в эфире»: закончились все in первого play (по умолчанию для кадра)
  settled+0.5      смещение
  out  out+0.2     маркер таймлайна: момент, когда воспроизведение впервые его проходит
  0..4             диапазон для check, filmstrip, video; по умолчанию — весь сценарий
  in  loop  out    там, где нужен диапазон, — сегмент: in — от play до settled, маркер —
                   до следующего маркера, out без маркера — от stop до конца
  --cue \"0 play; 125f stop\"   команды сценария в короткой форме (дополняют *.cues.ts)

Данные сцены (defineScene({ props, defaults })):
  --props data.json     значения из файла
  --set name=Иван       одно значение; тип приводится по схеме, json-пропсы — JSON

Общие флаги:
  --json           вывод одним JSON-объектом со схемой \"kine\": 1
  --mode dev|air   dev (по умолчанию): нарушение must — ошибка; air: применяются шаги else
  --bg \"#RRGGBB\"   подложка: рисуется под сценой и учитывается в проверке контраста
  --seed N         сид Math.random
  --now MS         значение Date.now

Бандл кэшируется в .kine/cache (KINE_CACHE_DIR — другой каталог, KINE_NO_CACHE=1 — без кэша).

Коды выхода: 0 успех, 1 ошибки в диагностиках, 2 неверный вызов, 3 сборка, 4 исключение JS, 5 сбой kine.";

const COMMANDS: &[&str] = &["render", "dump", "inspect", "check", "filmstrip", "video", "schema", "view"];

struct Args {
    rest: Vec<String>,
    flags: Vec<(String, Option<String>)>,
}

const VALUED: &[&str] = &[
    "-o", "--scale", "--bg", "--seed", "--now", "--mode", "--t", "--cue", "--n", "--cols", "--tile", "--codec", "--dir", "--port",
    "--props", "--set", "--path", "--depth",
];
const SWITCHES: &[&str] = &["--dump", "--json", "-h", "--help"];

impl Args {
    fn parse(raw: &[String]) -> Result<Self> {
        let mut rest = Vec::new();
        let mut flags = Vec::new();
        let mut it = raw.iter().cloned();
        while let Some(a) = it.next() {
            if VALUED.contains(&a.as_str()) {
                let v = it.next().with_context(|| format!("{a}: нет значения"))?;
                flags.push((a, Some(v)));
            } else if SWITCHES.contains(&a.as_str()) {
                flags.push((a, None));
            } else if a.starts_with("--") {
                bail!("неизвестный флаг {a}; список флагов — kine help");
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
        self.flags.iter().rev().find(|(k, _)| k == name).and_then(|(_, v)| v.as_deref())
    }

    fn values(&self, name: &str) -> Vec<&str> {
        self.flags.iter().filter(|(k, _)| k == name).filter_map(|(_, v)| v.as_deref()).collect()
    }

    fn number<T: std::str::FromStr>(&self, name: &str) -> Result<Option<T>> {
        self.value(name).map(|s| s.parse::<T>().map_err(|_| anyhow::anyhow!("{name}: ожидается число, получено {s:?}"))).transpose()
    }

    fn entry(&self) -> Result<PathBuf> {
        self.rest.get(1).map(PathBuf::from).context("не указан файл сцены")
    }
}

fn mode(args: &Args) -> Result<Mode> {
    Ok(match args.value("--mode") {
        None | Some("dev") => Mode::Dev,
        Some("air") => Mode::Air,
        Some(other) => bail!("--mode: ожидается dev или air, получено {other}"),
    })
}

/// Пропсы сцены из `--props` и `--set` для JS: `{ values, raw }`.
fn props(args: &Args) -> Result<Option<String>> {
    let mut values = serde_json::Map::new();
    if let Some(file) = args.value("--props") {
        let text = std::fs::read_to_string(file).with_context(|| format!("--props: не удалось прочитать {file}"))?;
        let v: serde_json::Value = serde_json::from_str(&text).with_context(|| format!("--props: {file} — не JSON"))?;
        match v {
            serde_json::Value::Object(m) => values = m,
            _ => bail!("--props: в {file} ожидается объект {{ \"имя\": значение }}"),
        }
    }
    let mut raw = serde_json::Map::new();
    for set in args.values("--set") {
        let Some((k, v)) = set.split_once('=') else { bail!("--set: ожидается имя=значение, получено {set:?}") };
        raw.insert(k.trim().to_string(), serde_json::Value::String(v.to_string()));
    }
    if values.is_empty() && raw.is_empty() {
        return Ok(None);
    }
    Ok(Some(serde_json::json!({ "values": values, "raw": raw }).to_string()))
}

fn run_options(args: &Args) -> Result<RunOptions> {
    let mut opts = RunOptions::default();
    if let Some(s) = args.number("--seed")? {
        opts.seed = s;
    }
    if let Some(n) = args.number("--now")? {
        opts.now_ms = n;
    }
    opts.props = props(args)?;
    Ok(opts)
}

fn eval_options(args: &Args) -> Result<EvalOptions> {
    let background = args.value("--bg").map(ops::parse_hex).transpose()?;
    Ok(EvalOptions { mode: mode(args)?, background })
}

fn cues(args: &Args) -> Result<Vec<CueArg>> {
    Ok(args.value("--cue").map(parse_cues).transpose()?.unwrap_or_default())
}

/// Результат команды: структура для `--json` и текст для терминала.
struct Done {
    exit: u8,
    entry: Option<String>,
    at: Option<At>,
    result: serde_json::Value,
    text: String,
    logs: Vec<Log>,
    next: Vec<String>,
    /// `--t` для строки воспроизведения.
    t: Option<String>,
}

impl Done {
    fn new<T: Serialize>(job: &Job, result: &T, text: String, errors: bool) -> Result<Done> {
        Ok(Done {
            exit: if errors { 1 } else { 0 },
            entry: Some(job.entry.to_string_lossy().replace('\\', "/")),
            at: None,
            result: output::tidy(serde_json::to_value(result)?),
            text,
            logs: Vec::new(),
            next: Vec::new(),
            t: None,
        })
    }
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let json = raw.iter().any(|a| a == "--json");
    let command = raw.iter().find(|a| !a.starts_with('-')).cloned().unwrap_or_else(|| "help".into());
    match run(&raw) {
        Ok(None) => ExitCode::SUCCESS,
        Ok(Some(done)) => {
            if json {
                let env = Envelope {
                    kine: output::SCHEMA,
                    command,
                    ok: true,
                    exit: done.exit,
                    entry: done.entry,
                    at: done.at,
                    result: Some(done.result),
                    error: None,
                    logs: done.logs,
                    next: done.next,
                    repro: Some(repro(&raw, done.t.as_deref())),
                };
                println!("{}", serde_json::to_string(&env).unwrap_or_default());
            } else {
                for log in &done.logs {
                    eprintln!("js {}: {}", log.level, log.message);
                }
                print!("{}", done.text);
                for n in &done.next {
                    eprintln!("→ {n}");
                }
                if done.exit == 1 {
                    eprintln!("repro: {}", repro(&raw, done.t.as_deref()));
                }
            }
            ExitCode::from(done.exit)
        }
        Err(e) => {
            let entry = Args::parse(&raw).ok().and_then(|a| a.rest.get(1).cloned());
            let (exit, env) = output::failure(&command, entry.as_deref(), &e);
            if json {
                println!("{}", serde_json::to_string(&env).unwrap_or_default());
            } else if let Some(err) = env.error.as_ref().filter(|x| x.kind == "build" && !x.diagnostics.is_empty()) {
                // Ошибки сборки — уже диагностики с местом в исходнике и строкой кода.
                for d in &err.diagnostics {
                    eprintln!("{d}");
                    if let Some(line) = d.detail.as_ref().and_then(|v| v.get("line")).and_then(|v| v.as_str()) {
                        eprintln!("    | {}", line.trim());
                    }
                }
            } else {
                eprintln!("error: {}", output::message(&e));
                for n in &env.next {
                    eprintln!("→ {n}");
                }
            }
            ExitCode::from(exit)
        }
    }
}

/// Строка, которая воспроизводит вызов: те же флаги, `--t` — разрешённое время.
fn repro(raw: &[String], t: Option<&str>) -> String {
    let mut out = vec!["kine".to_string()];
    let mut it = raw.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--json" => {}
            "--t" => {
                it.next();
            }
            _ => out.push(quote(a)),
        }
    }
    if let Some(t) = t {
        out.push("--t".into());
        out.push(quote(t));
    }
    out.join(" ")
}

/// Следующий шаг при ошибке в кадре: inspect узла в тот же момент.
fn next_for(job: &Job, diags: &[&Diagnostic], t: f64) -> Vec<String> {
    let entry = quote(&job.entry.to_string_lossy().replace('\\', "/"));
    diags
        .iter()
        .find(|d| d.level == Level::Error && !matches!(d.node.as_str(), "stage" | "build" | "js" | ""))
        .map(|d| vec![format!("kine inspect {entry} {} --t {}", quote(&d.node), num(t as f32))])
        .unwrap_or_default()
}

fn run(raw: &[String]) -> Result<Option<Done>> {
    let args = Args::parse(raw).context(Failure::Usage)?;
    let Some(cmd) = args.rest.first().map(String::as_str) else {
        println!("{USAGE}");
        return Ok(None);
    };
    if cmd == "help" || args.flag("-h") || args.flag("--help") {
        println!("{USAGE}");
        return Ok(None);
    }
    if !COMMANDS.contains(&cmd) {
        return Err(anyhow::anyhow!("неизвестная команда {cmd}; команды: {}", COMMANDS.join(", ")).context(Failure::Usage));
    }
    if cmd == "view" {
        view_cmd(&args)?;
        return Ok(None);
    }
    let (entry, run, cues, eval, spec) = (|| -> Result<_> {
        let spec = args.value("--t").map(time::parse).transpose().context("--t")?;
        Ok((args.entry()?, run_options(&args)?, cues(&args)?, eval_options(&args)?, spec))
    })()
    .context(Failure::Usage)?;
    if !entry.exists() {
        return Err(anyhow::anyhow!("сцена {} не найдена", entry.display()).context(Failure::Usage));
    }
    let job = Job::new(&entry, run, cues, eval)?;
    let mut done = match cmd {
        "render" | "dump" | "inspect" => {
            if let Some(TimeSpec::Range(..)) = spec {
                return Err(anyhow::anyhow!("--t: {cmd} принимает момент; диапазон — для check, filmstrip и video").context(Failure::Usage));
            }
            let t = job.moment(spec.as_ref())?;
            match cmd {
                "render" => {
                    let out = args.value("-o").map(PathBuf::from).unwrap_or_else(|| entry.with_extension("png"));
                    let scale = args.number("--scale").context(Failure::Usage)?.unwrap_or(1.0);
                    let r = ops::render(&job, t, &out, scale, args.flag("--dump"))?;
                    let mut done = Done::new(&job, &r, output::render_text(&r), r.counts.error > 0)?;
                    done.next = next_for(&job, &r.diagnostics.iter().collect::<Vec<_>>(), r.at.t);
                    done.at = Some(r.at);
                    done
                }
                "dump" => {
                    let depth = args.number("--depth").context(Failure::Usage)?;
                    let (d, at) = ops::dump(&job, t, args.value("--path"), depth)?;
                    let errors = d.diagnostics.iter().any(|x| x.level == Level::Error);
                    let mut done = Done::new(&job, &d, dump::text(&d), errors)?;
                    done.next = next_for(&job, &d.diagnostics.iter().collect::<Vec<_>>(), at.t);
                    done.at = Some(at);
                    done
                }
                _ => {
                    let path = args.rest.get(2).context("не указан адрес узла, например LowerThird#1/title; адреса — в kine dump").context(Failure::Usage)?;
                    let (i, at) = ops::inspect(&job, t, path)?;
                    let errors = i.diagnostics.iter().any(|x| x.level == Level::Error);
                    let mut done = Done::new(&job, &i, dump::inspect_text(&i), errors)?;
                    done.at = Some(at);
                    done
                }
            }
        }
        "check" | "filmstrip" | "video" => {
            let (from, to) = job.range(spec.as_ref())?;
            let mut done = match cmd {
                "check" => {
                    let c = ops::check(&job, from, to)?;
                    let mut done = Done::new(&job, &c, output::check_text(&c), c.counts.error > 0)?;
                    if let Some(first) = c.issues.iter().find(|i| i.diagnostic.level == Level::Error) {
                        done.next = next_for(&job, &[&first.diagnostic], first.first.t);
                    }
                    done
                }
                "filmstrip" => {
                    let o = ops::FilmstripOptions {
                        count: args.number("--n").context(Failure::Usage)?.unwrap_or(8),
                        cols: args.number("--cols").context(Failure::Usage)?.unwrap_or(4),
                        tile: args.number("--tile").context(Failure::Usage)?.unwrap_or(480.0),
                        dumps: args.flag("--dump"),
                    };
                    let out = args.value("-o").map(PathBuf::from).unwrap_or_else(|| strip_path(&entry));
                    let f = ops::filmstrip(&job, from, to, &out, &o)?;
                    Done::new(&job, &f, output::filmstrip_text(&f), f.counts.error > 0)?
                }
                _ => {
                    let out = args.value("-o").map(PathBuf::from).unwrap_or_else(|| entry.with_extension("mov"));
                    let codec = match args.value("--codec") {
                        Some(c) => kine_cli::video::Codec::parse(c),
                        None => kine_cli::video::Codec::for_path(&out),
                    }
                    .context(Failure::Usage)?;
                    let scale = args.number("--scale").context(Failure::Usage)?.unwrap_or(1.0);
                    let v = ops::video(&job, from, to, &out, codec, scale)?;
                    Done::new(&job, &v, output::video_text(&v), v.frames_with_errors > 0)?
                }
            };
            done.t = Some(format!("{}..{}", num(from as f32), num(to as f32)));
            done
        }
        "schema" => {
            let s = ops::schema(&job)?;
            Done::new(&job, &s, output::schema_text(&s), false)?
        }
        _ => unreachable!(),
    };
    if let Some(at) = done.at
        && done.t.is_none()
    {
        done.t = Some(num(at.t as f32));
    }
    done.logs = job.logs.borrow().iter().map(Log::from).collect();
    Ok(Some(done))
}

fn view_cmd(args: &Args) -> Result<()> {
    let (run, extra, eval, port) = (|| -> Result<_> { Ok((run_options(args)?, cues(args)?, eval_options(args)?, args.number("--port")?.unwrap_or(4747))) })()
        .context(Failure::Usage)?;
    let opts = kine_cli::view::ViewOptions {
        entry: args.rest.get(1).map(PathBuf::from),
        dir: PathBuf::from(args.value("--dir").unwrap_or("out")),
        port,
        run,
        extra,
        eval,
    };
    if let Some(entry) = &opts.entry
        && !entry.exists()
    {
        return Err(anyhow::anyhow!("сцена {} не найдена", entry.display()).context(Failure::Usage));
    }
    kine_cli::view::serve(opts)
}

fn strip_path(entry: &Path) -> PathBuf {
    entry.with_extension("strip.png")
}
