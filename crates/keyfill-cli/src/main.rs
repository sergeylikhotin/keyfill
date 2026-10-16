use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use keyfill_cli::config::{self, Config};
use keyfill_cli::help;
use keyfill_cli::ops::{self, At, Job, Outcome};
use keyfill_cli::output::{self, Envelope, Log, quote};
use keyfill_cli::session::{CueArg, parse_cues};
use keyfill_cli::suite::{self, SuiteOptions};
use keyfill_cli::time::{self, TimeSpec};
use keyfill_cli::video::Codec;
use keyfill_core::dump;
use keyfill_core::{EvalOptions, Mode};
use keyfill_js::RunOptions;
use serde::Serialize;
use serde_json::value::RawValue;

const USAGE: &str = "\
keyfill — рендер TSX-сцен без браузера

Увидеть:
  keyfill render <entry> [-o out.png] [--scale N] [--dump]
  keyfill dump <entry> [--path <узел>] [--depth N]
                                      дерево сцены: геометрия, движение, диагностики
  keyfill inspect <entry> <узел>         один узел: значения, их происхождение, движение, исходник
  keyfill filmstrip <entry> [--t a..b] [-o strip.png] [--n 8] [--cols 4] [--tile 480] [--dump]
                                      раскадровка: n моментов диапазона одной картинкой
  keyfill video <entry> [--t a..b] [-o out.mov] [--codec prores|qtrle|vp9|h264|png] [--scale N]
                                      видео через ffmpeg: .mov — ProRes 4444 с альфой, .webm — VP9
                                      с альфой, .mp4 — H.264 на подложке, каталог — PNG-кадры
  keyfill view [entry] [--dir out] [--port 4747]
                                      веб-просмотрщик: галерея выводов и живая сцена
Проверить:
  keyfill check <entry> [--t a..b]       нарушения must и правил на каждом кадре диапазона
                                      (по умолчанию — весь сценарий)
  keyfill test [файлы|каталоги] [--update] [--name часть]
                                      случаи *.keyfill.test.ts: clean и снапшоты дампа и пикселей
  keyfill perf <entry> [--t a..b] [--scale N]
                                      время кадра по стадиям против бюджета 1/fps
Понять:
  keyfill schema <entry>                 пропсы сцены, компоненты, части, команды, маркеры, сценарий;
                                      у канала — каталог и JSON Schema пропсов (--json)
  keyfill explain [правило]              что значит диагностика и как её исправить
  keyfill docs [тема]                    справка: время, сцена, пропсы, канал, look, раскладка, must,
                                      движение, тесты
Создать и обслужить:
  keyfill new <имя> [--template lower-third|blank]
                                      каталог сцены из шаблона; tsconfig, keyfill.config.ts и шрифт —
                                      если их ещё нет
  keyfill doctor                         node, @keyfill/core, esbuild, ffmpeg, конфиг и пробная сборка
  keyfill cache [clean]                  кэш бандлов: размер или очистка

<entry> — сцена (defineScene), сценарий *.cues.ts (cues), канал (defineChannel), журнал
команд канала *.journal.jsonl или имя сцены из keyfill.config.ts. Без сценария сцена получает
play в 0; канал без журнала пуст (keyfill docs channel).

Время (одна грамматика для --t и --cue):
  1.2  60f         секунды или кадры
  settled          кадр «в эфире»: закончились все in первого play (по умолчанию для кадра)
  settled+0.5      смещение
  out  out+0.2     маркер таймлайна: момент, когда воспроизведение впервые его проходит
  0..4             диапазон для check, filmstrip, video, perf; по умолчанию — весь сценарий
  in  loop  out    там, где нужен диапазон, — сегмент: in — от play до settled, маркер —
                   до следующего маркера, out без маркера — от stop до конца
  --cue \"0 play; 125f stop\"   команды сценария в короткой форме (дополняют *.cues.ts)

Данные сцены (defineScene({ props, defaults })):
  --props data.json     значения из файла
  --set name=Иван       одно значение; тип приводится по схеме, списки и объекты — JSON

Общие флаги:
  --json           вывод одним JSON-объектом со схемой \"keyfill\": 1
  --mode dev|air   dev (по умолчанию): нарушение must — ошибка; air: применяются шаги else
  --bg \"#RRGGBB\"   подложка: рисуется под сценой и учитывается в проверке контраста
  --seed N         сид Math.random
  --now MS         значение Date.now

keyfill.config.ts (ищется вверх от каталога запуска) задаёт значения по умолчанию: mode, bg, seed,
now, каталог вывода out и именованные сцены; флаги их перекрывают. Подробно — keyfill docs config.

Бандл кэшируется в .keyfill/cache (KEYFILL_CACHE_DIR — другой каталог, KEYFILL_NO_CACHE=1 — без кэша).

keyfill --version — версия.

Коды выхода: 0 успех, 1 ошибки в диагностиках или упавшие тесты, 2 неверный вызов, 3 сборка,
4 исключение JS, 5 сбой keyfill.";

const COMMANDS: &[&str] =
    &["render", "dump", "inspect", "check", "filmstrip", "video", "view", "test", "perf", "schema", "explain", "docs", "new", "doctor", "cache"];
/// Команды, которые не принимают сцену вторым словом.
const SCENELESS: &[&str] = &["test", "explain", "docs", "new", "doctor", "cache"];

struct Args {
    rest: Vec<String>,
    flags: Vec<(String, Option<String>)>,
}

const VALUED: &[&str] = &[
    "-o", "--scale", "--bg", "--seed", "--now", "--mode", "--t", "--cue", "--n", "--cols", "--tile", "--codec", "--dir", "--port",
    "--props", "--set", "--path", "--depth", "--name", "--template",
];
const SWITCHES: &[&str] = &["--dump", "--json", "--update", "-h", "--help", "-V", "--version"];

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
                bail!("неизвестный флаг {a}; список флагов — keyfill help");
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

    /// Сцена: путь или имя из `keyfill.config.ts`.
    fn entry(&self, config: &Config) -> Result<PathBuf> {
        config.entry(self.rest.get(1).context("не указан файл сцены")?)
    }
}

fn mode(args: &Args, config: &Config) -> Result<Mode> {
    Ok(match args.value("--mode").or(config.mode.as_deref()) {
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

fn run_options(args: &Args, config: &Config) -> Result<RunOptions> {
    let mut opts = RunOptions::default();
    if let Some(s) = args.number("--seed")?.or(config.seed) {
        opts.seed = s;
    }
    if let Some(n) = args.number("--now")?.or(config.now) {
        opts.now_ms = n;
    }
    opts.props = props(args)?;
    Ok(opts)
}

fn eval_options(args: &Args, config: &Config) -> Result<EvalOptions> {
    let background = args.value("--bg").or(config.bg.as_deref()).map(ops::parse_hex).transpose()?;
    Ok(EvalOptions { mode: mode(args, config)?, background })
}

fn cues(args: &Args) -> Result<Vec<CueArg>> {
    Ok(args.value("--cue").map(parse_cues).transpose()?.unwrap_or_default())
}

/// Что сделать: команда с уже разобранными и проверенными флагами.
enum Command {
    Render { out: PathBuf, scale: f32, dump: bool },
    Dump { path: Option<String>, depth: Option<usize> },
    Inspect { path: String },
    Check,
    Filmstrip { out: PathBuf, opts: ops::FilmstripOptions },
    Video { out: PathBuf, codec: Codec, scale: f32 },
    Perf { scale: f32 },
    Schema,
}

/// Разобранный вызов. Любая ошибка разбора — неверный вызов (код 2).
struct Request {
    entry: PathBuf,
    run: RunOptions,
    cues: Vec<CueArg>,
    eval: EvalOptions,
    time: Option<TimeSpec>,
    command: Command,
}

/// Положительное число из флага: масштаб, ширина плитки, число кадров.
fn positive<T: std::str::FromStr + PartialOrd + Default + Copy + std::fmt::Display>(args: &Args, name: &str, default: T) -> Result<T> {
    let v = args.number(name)?.unwrap_or(default);
    // NaN несравнимо с нулём и тоже отвергается.
    if v.partial_cmp(&T::default()) != Some(std::cmp::Ordering::Greater) {
        bail!("{name}: ожидается число больше 0, получено {v}");
    }
    Ok(v)
}

/// Файл результата: `-o` или путь по умолчанию (каталог `out` конфига или рядом со сценой).
fn output(args: &Args, config: &Config, entry: &Path, ext: &str) -> Result<PathBuf> {
    match args.value("-o") {
        Some(o) => Ok(PathBuf::from(o)),
        None => config.output(entry, ext),
    }
}

fn request(args: &Args, cmd: &str, config: &Config) -> Result<Request> {
    let entry = args.entry(config)?;
    let time = args.value("--t").map(time::parse).transpose().context("--t")?;
    let takes_range = matches!(cmd, "check" | "filmstrip" | "video" | "perf");
    if !takes_range && let Some(TimeSpec::Range(..)) = time {
        bail!("--t: {cmd} принимает момент; диапазон — для check, filmstrip, video и perf");
    }
    let command = match cmd {
        "render" => Command::Render { out: output(args, config, &entry, "png")?, scale: positive(args, "--scale", 1.0)?, dump: args.flag("--dump") },
        "dump" => Command::Dump { path: args.value("--path").map(String::from), depth: args.number("--depth")? },
        "inspect" => Command::Inspect { path: args.rest.get(2).cloned().context("не указан адрес узла, например LowerThird#1/title; адреса — в keyfill dump")? },
        "check" => Command::Check,
        "filmstrip" => Command::Filmstrip {
            out: output(args, config, &entry, "strip.png")?,
            opts: ops::FilmstripOptions {
                count: positive(args, "--n", 8)?,
                cols: positive(args, "--cols", 4)?,
                tile: positive(args, "--tile", 480.0)?,
                dumps: args.flag("--dump"),
            },
        },
        "video" => {
            let out = output(args, config, &entry, "mov")?;
            let codec = match args.value("--codec") {
                Some(c) => Codec::parse(c)?,
                None => Codec::for_path(&out)?,
            };
            Command::Video { out, codec, scale: positive(args, "--scale", 1.0)? }
        }
        "perf" => Command::Perf { scale: positive(args, "--scale", 1.0)? },
        "schema" => Command::Schema,
        _ => unreachable!("команда проверена в run"),
    };
    Ok(Request { entry, run: run_options(args, config)?, cues: cues(args)?, eval: eval_options(args, config)?, time, command })
}

/// Результат команды: структура для `--json` и текст для терминала.
struct Done {
    exit: u8,
    entry: Option<String>,
    at: Option<At>,
    result: Box<RawValue>,
    text: String,
    logs: Vec<Log>,
    next: Vec<String>,
    /// `--t` для строки воспроизведения: разрешённый момент или диапазон.
    t: Option<String>,
    /// `keyfill.config.ts`, если он есть.
    config: Option<String>,
}

impl Done {
    /// Результат операции над сценой: подсказка `inspect` с адресом сцены, вывод console.*.
    fn new<T: Serialize>(job: &Job, o: Outcome<T>, text: impl FnOnce(&T) -> String) -> Result<Done> {
        let entry = keyfill_cli::slash(&job.entry);
        let inspect = o.inspect.as_ref().map(|(node, t)| format!("keyfill inspect {} {} --t {t}", quote(&entry), quote(node)));
        let mut done = Done::plain(o, text)?;
        done.next.splice(0..0, inspect);
        done.logs = job.logs.borrow().iter().map(Log::from).collect();
        done.entry = Some(entry);
        Ok(done)
    }

    /// Результат команды без сцены (test, explain, docs) и общая часть `new`.
    fn plain<T: Serialize>(o: Outcome<T>, text: impl FnOnce(&T) -> String) -> Result<Done> {
        let t = match (o.at, o.range) {
            (Some(at), _) => Some(at.t.to_string()),
            (_, Some((from, to))) => Some(format!("{from}..{to}")),
            _ => None,
        };
        let mut next = o.next;
        next.extend(o.explain.map(|rule| format!("keyfill explain {rule}")));
        Ok(Done {
            exit: u8::from(o.errors),
            text: text(&o.result),
            result: serde_json::value::to_raw_value(&o.result)?,
            entry: None,
            at: o.at,
            logs: Vec::new(),
            next,
            t,
            config: None,
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
                    keyfill: output::SCHEMA,
                    command,
                    ok: true,
                    exit: done.exit,
                    entry: done.entry,
                    config: done.config,
                    at: done.at,
                    result: Some(done.result),
                    error: None,
                    logs: done.logs,
                    next: done.next,
                    repro: Some(repro(&raw, done.t.as_deref())),
                };
                out(&format!("{}\n", serde_json::to_string(&env).unwrap_or_default()));
            } else {
                for log in &done.logs {
                    eprintln!("js {}: {}", log.level, log.message);
                }
                out(&done.text);
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
            // Второе слово — сцена, кроме команд, которые сцену не принимают.
            let entry = Args::parse(&raw)
                .ok()
                .filter(|a| !a.rest.first().is_some_and(|c| SCENELESS.contains(&c.as_str())))
                .and_then(|a| a.rest.get(1).cloned());
            let (exit, env) = output::failure(&command, entry.as_deref(), &e);
            if json {
                out(&format!("{}\n", serde_json::to_string(&env).unwrap_or_default()));
            } else if let Some(err) = env.error.as_ref().filter(|x| x.kind == "build" && !x.diagnostics.is_empty()) {
                // Ошибки сборки — уже диагностики с местом в исходнике и строкой кода.
                for d in &err.diagnostics {
                    eprintln!("{d}");
                    if let Some(line) = d.detail.as_ref().and_then(|v| v.get("line")).and_then(|v| v.as_str()) {
                        eprintln!("    | {}", line.trim());
                    }
                }
            } else {
                eprintln!("error: {e:#}");
                for n in &env.next {
                    eprintln!("→ {n}");
                }
            }
            ExitCode::from(exit)
        }
    }
}

/// Вывод в stdout без паники на закрытом канале (`keyfill dump … | head`).
fn out(text: &str) {
    use std::io::Write as _;
    let _ = std::io::stdout().lock().write_all(text.as_bytes());
}

/// Строка, которая воспроизводит вызов: те же флаги, `--t` — разрешённое время.
fn repro(raw: &[String], t: Option<&str>) -> String {
    let mut out = vec!["keyfill".to_string()];
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

/// Ошибка разбора вызова — неверный вызов (код 2).
fn bad_call(e: anyhow::Error) -> anyhow::Error {
    keyfill_cli::usage(format!("{e:#}"))
}

fn run(raw: &[String]) -> Result<Option<Done>> {
    let args = Args::parse(raw).map_err(bad_call)?;
    if args.flag("--version") || args.flag("-V") {
        out(&format!("keyfill {} ({}-{})\n", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH));
        return Ok(None);
    }
    let Some(cmd) = args.rest.first().map(String::as_str) else {
        out(&format!("{USAGE}\n"));
        return Ok(None);
    };
    if cmd == "help" || args.flag("-h") || args.flag("--help") {
        out(&format!("{USAGE}\n"));
        return Ok(None);
    }
    if !COMMANDS.contains(&cmd) {
        return Err(keyfill_cli::usage(format!("неизвестная команда {cmd}; команды: {}", COMMANDS.join(", "))));
    }
    // Справка и обслуживание не зависят от конфига: doctor проверяет его сам.
    match cmd {
        "explain" => return explain(&args).map(Some),
        "docs" => return docs(&args).map(Some),
        "new" => return new_cmd(&args).map(Some),
        "doctor" => {
            let report = keyfill_cli::doctor::run();
            let errors = report.errors();
            return Done::plain(Outcome { errors, ..Outcome::new(report) }, output::doctor_text).map(Some);
        }
        "cache" => {
            let clean = match args.rest.get(1).map(String::as_str) {
                None => false,
                Some("clean") => true,
                Some(other) => return Err(keyfill_cli::usage(format!("keyfill cache: {other}? есть keyfill cache и keyfill cache clean"))),
            };
            return Done::plain(Outcome::new(keyfill_cli::doctor::cache(clean)?), output::cache_text).map(Some);
        }
        _ => {}
    }
    let config = config::load()?;
    let shown = config.shown();
    let done = match cmd {
        "view" => {
            view_cmd(&args, &config)?;
            return Ok(None);
        }
        "test" => test_cmd(&args, &config)?,
        _ => scene_cmd(&args, cmd, &config)?,
    };
    Ok(Some(Done { config: shown, ..done }))
}

fn scene_cmd(args: &Args, cmd: &str, config: &Config) -> Result<Done> {
    let req = request(args, cmd, config).map_err(bad_call)?;
    let job = Job::new(&req.entry, req.run, req.cues, req.eval)?;
    let spec = req.time.as_ref();
    let moment = match spec {
        Some(TimeSpec::Moment(m)) => Some(m),
        _ => None,
    };
    match &req.command {
        Command::Render { out, scale, dump } => Done::new(&job, ops::render(&job, moment, out, *scale, *dump)?, output::render_text),
        Command::Dump { path, depth } => Done::new(&job, ops::dump(&job, moment, path.as_deref(), *depth)?, dump::text),
        Command::Inspect { path } => Done::new(&job, ops::inspect(&job, moment, path)?, dump::inspect_text),
        Command::Check => Done::new(&job, ops::check(&job, spec)?, output::check_text),
        Command::Filmstrip { out, opts } => Done::new(&job, ops::filmstrip(&job, spec, out, opts)?, output::filmstrip_text),
        Command::Video { out, codec, scale } => Done::new(&job, ops::video(&job, spec, out, *codec, *scale)?, output::video_text),
        Command::Perf { scale } => Done::new(&job, ops::perf(&job, spec, *scale)?, output::perf_text),
        Command::Schema => Done::new(&job, ops::schema(&job)?, output::schema_text),
    }
}

fn new_cmd(args: &Args) -> Result<Done> {
    let name = args.rest.get(1).ok_or_else(|| keyfill_cli::usage("keyfill new <имя>: не указано имя сцены"))?;
    let created = keyfill_cli::scaffold::create(name, args.value("--template").unwrap_or("lower-third"))?;
    let entry = quote(&created.entry);
    let mut o = Outcome::new(created);
    o.next = vec![format!("keyfill check {entry}"), format!("keyfill test {}", quote(&o.result.dir))];
    Done::plain(o, output::new_text)
}

fn explain(args: &Args) -> Result<Done> {
    let Some(id) = args.rest.get(1) else { return Done::plain(Outcome::new(help::rules()), output::rules_text) };
    let Some(rule) = help::rule(id) else {
        let near = help::similar(id);
        let hint = if near.is_empty() { "список — keyfill explain".to_string() } else { format!("похожие: {}", near.join(", ")) };
        return Err(keyfill_cli::usage(format!("правила {id} нет; {hint}")));
    };
    Done::plain(Outcome::new(rule), output::rule_text)
}

fn docs(args: &Args) -> Result<Done> {
    let Some(name) = args.rest.get(1) else { return Done::plain(Outcome::new(help::topics()), output::topics_text) };
    let Some(topic) = help::topic(name) else {
        let names: Vec<String> = help::topics().into_iter().map(|t| t.topic).collect();
        return Err(keyfill_cli::usage(format!("темы {name} нет; есть: {}", names.join(", "))));
    };
    Done::plain(Outcome::new(topic), output::topic_text)
}

/// `keyfill test` идёт из каталога конфига, если он есть: пути в дампах и снапшотах — от него,
/// и результат не зависит от того, откуда запущен keyfill.
fn test_cmd(args: &Args, config: &Config) -> Result<Done> {
    let parse = || -> Result<SuiteOptions> {
        let paths = args.rest[1..].iter().map(|p| std::path::absolute(p).map_err(Into::into)).collect::<Result<Vec<PathBuf>>>()?;
        Ok(SuiteOptions { paths, name: args.value("--name").map(String::from), update: args.flag("--update"), run: run_options(args, config)? })
    };
    let mut opts = parse().map_err(bad_call)?;
    if let Some(dir) = config.file.as_deref().and_then(Path::parent) {
        std::env::set_current_dir(dir)?;
    }
    let cwd = std::env::current_dir()?;
    for p in &mut opts.paths {
        if let Ok(rel) = p.strip_prefix(&cwd) {
            *p = if rel.as_os_str().is_empty() { PathBuf::from(".") } else { rel.to_path_buf() };
        }
    }
    let mut o = suite::run(&opts)?;
    let changed = o.result.failing_snapshots();
    if !changed.is_empty() {
        let files: Vec<String> = changed.iter().map(|f| quote(f)).collect();
        o.next.push(format!("keyfill test {} --update", files.join(" ")));
    }
    o.explain = o.result.first_error_rule();
    Done::plain(o, output::test_text)
}

fn view_cmd(args: &Args, config: &Config) -> Result<()> {
    let parse = || -> Result<keyfill_cli::view::ViewOptions> {
        Ok(keyfill_cli::view::ViewOptions {
            entry: args.rest.get(1).map(|e| config.entry(e)).transpose()?,
            dir: args.value("--dir").map(PathBuf::from).or_else(|| config.out.clone()).unwrap_or_else(|| PathBuf::from("out")),
            port: args.number("--port")?.unwrap_or(4747),
            run: run_options(args, config)?,
            extra: cues(args)?,
            eval: eval_options(args, config)?,
        })
    };
    let opts = parse().map_err(bad_call)?;
    keyfill_cli::view::serve(opts)
}
