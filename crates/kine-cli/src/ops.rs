//! Операции агента: render, dump, inspect, check, filmstrip, video, schema, perf.
//!
//! Один слой для CLI и (позже) `kine serve` и MCP. Каждая операция возвращает `Outcome`:
//! структуру результата и то, что нужно конверту (момент, диапазон, ошибки, следующий шаг).
//! Текст для терминала строится из той же структуры (`output.rs`), поэтому `--json` и текст
//! не расходятся.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
pub use kine_core::diag::Counts;
use kine_core::diag::{Diagnostic, Level};
use kine_core::dump::{self, Dump, Inspect};
use kine_core::look::Rgba;
use kine_core::motion::component_of;
use kine_core::scene::{ComponentInfo, PropInfo};
use kine_core::timeline::LoopDef;
use kine_core::{EvalOptions, Frame, Scene};
use kine_js::{LogLine, RunOptions};
use kine_render::{Png, RenderOptions, Renderer};
use serde::Serialize;

use crate::session::CueArg;
use crate::time::{self, Moment, Offset, TimeSpec};
use crate::{Bundle, Session, filmstrip, video};

/// Всё, что нужно, чтобы построить кадр: сцена, данные, сценарий, режим.
pub struct Job {
    pub entry: PathBuf,
    pub bundle: Bundle,
    pub run: RunOptions,
    pub cues: Vec<CueArg>,
    pub eval: EvalOptions,
    /// Вывод console.* сцены из последней сессии операции.
    pub logs: std::cell::RefCell<Vec<LogLine>>,
}

/// Момент, в котором построен кадр.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct At {
    pub t: f64,
    pub frame: u64,
}

/// Результат операции и то, что нужно конверту вокруг него.
pub struct Outcome<T> {
    pub result: T,
    /// Момент кадра: render, dump, inspect.
    pub at: Option<At>,
    /// Диапазон: check, filmstrip, video.
    pub range: Option<(f64, f64)>,
    /// В результате есть ошибки диагностики: код выхода 1.
    pub errors: bool,
    /// Узел первой ошибки и её момент — для подсказки `kine inspect`.
    pub inspect: Option<(String, f64)>,
    /// Правило первой ошибки — для подсказки `kine explain`.
    pub explain: Option<String>,
    /// Другие команды следующего шага, уже готовые.
    pub next: Vec<String>,
}

impl<T> Outcome<T> {
    pub fn new(result: T) -> Outcome<T> {
        Outcome { result, at: None, range: None, errors: false, inspect: None, explain: None, next: Vec::new() }
    }

    /// Ошибки кадра в момент `at`: код выхода и подсказки `inspect` и `explain`.
    fn frame(result: T, at: At, scene: &Scene, diags: &[Diagnostic]) -> Outcome<T> {
        Outcome {
            errors: Counts::of(diags).error > 0,
            inspect: inspect_target(scene, diags).map(|node| (node, at.t)),
            explain: first_error(diags),
            at: Some(at),
            ..Outcome::new(result)
        }
    }
}

/// Правило первой ошибки.
pub fn first_error<'a>(diags: impl IntoIterator<Item = &'a Diagnostic>) -> Option<String> {
    diags.into_iter().find(|d| d.level == Level::Error).map(|d| d.rule.clone())
}

/// Первая ошибка, привязанная к узлу сцены: диагностики сборки, JS и таймлайнов адресуют
/// не узлы, и `kine inspect` для них бесполезен.
fn inspect_target<'a>(scene: &Scene, diags: impl IntoIterator<Item = &'a Diagnostic>) -> Option<String> {
    diags.into_iter().find(|d| d.level == Level::Error && scene.find(&d.node).is_some()).map(|d| d.node.clone())
}

impl Job {
    pub fn new(entry: &Path, run: RunOptions, cues: Vec<CueArg>, eval: EvalOptions) -> Result<Job> {
        Ok(Job::with_bundle(entry, crate::bundle(entry)?, run, cues, eval))
    }

    /// Задача на уже собранном бандле: случаи одного файла тестов делят его сборку.
    pub fn with_bundle(entry: &Path, bundle: Bundle, run: RunOptions, cues: Vec<CueArg>, eval: EvalOptions) -> Job {
        Job { entry: entry.to_path_buf(), bundle, run, cues, eval, logs: Default::default() }
    }

    pub fn session(&self) -> Result<Session> {
        Session::open(&self.bundle, &self.run, &self.cues)
    }

    /// Запоминает console.* сессии, которой операция построила результат.
    fn keep(&self, session: &Session) {
        *self.logs.borrow_mut() = session.logs.clone();
    }

    /// Диапазон из `--t`: `a..b`, имя сегмента (`in`, `out`, маркер таймлайна) или один
    /// момент; без `--t` — весь сценарий. Границы разрешаются на той же сессии, которая
    /// потом строит кадры: если она ушла дальше начала диапазона, то перемотается сама.
    pub fn range(&self, spec: Option<&TimeSpec>) -> Result<(Session, f64, f64)> {
        let mut s = self.session()?;
        let (from, to) = match spec {
            Some(TimeSpec::Range(a, b)) => (time::resolve(a, &mut s)?, time::resolve(b, &mut s)?),
            Some(TimeSpec::Moment(Moment::Marker(name, Offset::Seconds(o)))) if *o == 0.0 => time::segment(name, &mut s)?,
            Some(TimeSpec::Moment(m)) => {
                let t = time::resolve(m, &mut s)?;
                (t, t)
            }
            None => (0.0, s.end_time()?),
        };
        if to < from {
            return Err(crate::usage(format!("--t: конец диапазона {to} раньше начала {from}")));
        }
        Ok((s, from, to))
    }
}

/// Файл результата по `-o`: каталог, которого нет, — ошибка вызова, а не сбой kine.
fn write_output(out: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(out, bytes).map_err(|e| crate::usage(format!("не удалось записать {}: {e}", out.display())))
}

/// Кадр в момент из `--t` (без него — `settled`, кадр «в эфире»), на границе кадра.
/// Секунды и кадры округляются к ближайшему кадру, а `settled` и маркеры — вверх: кадр
/// не должен оказаться раньше события, которое он показывает.
pub fn frame(job: &Job, moment: Option<&Moment>) -> Result<(Session, Frame, At)> {
    let mut s = job.session()?;
    let t = match moment {
        Some(m) => {
            let t = time::resolve(m, &mut s)?;
            if matches!(m, Moment::At(_)) { s.nearest_frame(t) } else { s.snap(t) }
        }
        None => s.settled_time()?,
    };
    let frame = s.frame_at(t, job.eval)?;
    let at = At { t, frame: s.frame_index(t) };
    job.keep(&s);
    Ok((s, frame, at))
}

// ─── render ─────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct Rendered {
    pub file: String,
    /// Размер картинки в пикселях.
    pub size: [u32; 2],
    pub at: At,
    pub counts: Counts,
    /// Диагностики кадра; полный дамп — `--dump`.
    pub diagnostics: Vec<Diagnostic>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dump: Option<Dump>,
}

pub fn render(job: &Job, moment: Option<&Moment>, out: &Path, scale: f32, with_dump: bool) -> Result<Outcome<Rendered>> {
    let (session, frame, at) = frame(job, moment)?;
    let opts = RenderOptions { scale, background: job.eval.background };
    let mut renderer = Renderer::default();
    let pixmap = renderer.render(&frame, &opts)?;
    let size = [u32::from(pixmap.width()), u32::from(pixmap.height())];
    write_output(out, &kine_render::png(pixmap, Png::Small)?)?;
    let dump = dump::build(&session.scene, &frame);
    let result = Rendered {
        file: crate::slash(out),
        size,
        at,
        counts: Counts::of(&dump.diagnostics),
        diagnostics: dump.diagnostics.clone(),
        dump: with_dump.then_some(dump),
    };
    Ok(Outcome::frame(result, at, &session.scene, &frame.diagnostics.0))
}

// ─── dump и inspect ─────────────────────────────────────────────────────────

pub fn dump(job: &Job, moment: Option<&Moment>, path: Option<&str>, depth: Option<usize>) -> Result<Outcome<Dump>> {
    let (session, frame, at) = frame(job, moment)?;
    let mut d = dump::build(&session.scene, &frame);
    d.filter(path, depth).map_err(crate::usage)?;
    let diags = d.diagnostics.clone();
    Ok(Outcome::frame(d, at, &session.scene, &diags))
}

pub fn inspect(job: &Job, moment: Option<&Moment>, path: &str) -> Result<Outcome<Inspect>> {
    let (session, frame, at) = frame(job, moment)?;
    match dump::inspect_build(&session.scene, &frame, path) {
        Some(i) => {
            let diags = i.diagnostics.clone();
            // Подсказка `inspect` из inspect самого узла ничего не добавит.
            Ok(Outcome { inspect: None, ..Outcome::frame(i, at, &session.scene, &diags) })
        }
        None => {
            let known = dump::build(&session.scene, &frame).paths();
            let near: Vec<&str> = known.iter().map(String::as_str).filter(|p| p.contains(path) || path.contains(p)).take(5).collect();
            let hint = if near.is_empty() { "адреса узлов есть в `kine dump`".to_string() } else { format!("похожие: {}", near.join(", ")) };
            Err(crate::usage(format!("узел {path} не найден; {hint}")))
        }
    }
}

// ─── check ──────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct Check {
    pub from: f64,
    pub to: f64,
    /// Сколько кадров проверено.
    pub frames: usize,
    pub counts: Counts,
    pub issues: Vec<Issue>,
}

/// Нарушение за диапазон: одна запись на правило и узел, с первым и последним кадром.
#[derive(Debug, Serialize)]
pub struct Issue {
    #[serde(flatten)]
    pub diagnostic: Diagnostic,
    pub first: At,
    pub last: At,
    /// В скольких кадрах встретилось.
    pub frames: usize,
}

/// Все нарушения `must` и встроенных правил на каждом кадре диапазона.
pub fn check(job: &Job, spec: Option<&TimeSpec>) -> Result<Outcome<Check>> {
    let (mut session, from, to) = job.range(spec)?;
    session.labels = false;
    let frames = session.frames(from, to);
    let mut issues: Vec<Issue> = Vec::new();
    let mut index: BTreeMap<(Level, String, String, Option<String>), usize> = BTreeMap::new();
    for n in frames.clone() {
        let t = session.frame_time(n);
        let frame = session.frame_at(t, job.eval)?;
        let at = At { t, frame: n };
        for d in frame.diagnostics.0 {
            let key = (d.level, d.rule.clone(), d.node.clone(), d.source.clone());
            match index.get(&key) {
                Some(&i) => {
                    issues[i].last = at;
                    issues[i].frames += 1;
                }
                None => {
                    index.insert(key, issues.len());
                    issues.push(Issue { diagnostic: d, first: at, last: at, frames: 1 });
                }
            }
        }
    }
    job.keep(&session);
    issues.sort_by_key(|i| std::cmp::Reverse(i.diagnostic.level));
    let counts = Counts::of(issues.iter().map(|i| &i.diagnostic));
    let inspect = inspect_target(&session.scene, issues.iter().map(|i| &i.diagnostic))
        .and_then(|node| issues.iter().find(|i| i.diagnostic.node == node).map(|i| (node, i.first.t)));
    let explain = first_error(issues.iter().map(|i| &i.diagnostic));
    let result = Check { from, to, frames: frames.count(), counts, issues };
    Ok(Outcome { range: Some((from, to)), errors: counts.error > 0, inspect, explain, ..Outcome::new(result) })
}

// ─── filmstrip и video ──────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct Filmstrip {
    pub file: String,
    pub frames: Vec<At>,
    pub counts: Counts,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dumps: Vec<Dump>,
}

pub struct FilmstripOptions {
    pub count: usize,
    pub cols: usize,
    pub tile: f32,
    pub dumps: bool,
}

pub fn filmstrip(job: &Job, spec: Option<&TimeSpec>, out: &Path, o: &FilmstripOptions) -> Result<Outcome<Filmstrip>> {
    let (mut session, from, to) = job.range(spec)?;
    let fopts = filmstrip::Options { from, to, count: o.count, cols: o.cols, tile: o.tile, background: job.eval.background };
    let times = filmstrip::times(&session, &fopts);
    let strip = filmstrip::build(&mut session, &times, job.eval, &fopts)?;
    let png = kine_render::execute_png(&strip.display, &RenderOptions { scale: 1.0, background: None })?;
    write_output(out, &png)?;
    let counts = Counts::of(strip.frames.iter().flat_map(|f| &f.diagnostics.0));
    job.keep(&session);
    let result = Filmstrip {
        file: crate::slash(out),
        frames: times.iter().map(|&t| At { t, frame: session.frame_index(t) }).collect(),
        counts,
        dumps: if o.dumps { strip.dumps } else { Vec::new() },
    };
    Ok(Outcome { range: Some((from, to)), errors: counts.error > 0, ..Outcome::new(result) })
}

#[derive(Debug, Serialize)]
pub struct Video {
    pub file: String,
    pub codec: &'static str,
    pub from: f64,
    pub to: f64,
    pub frames: usize,
    /// Кадров с ошибками диагностики.
    pub frames_with_errors: usize,
}

pub fn video(job: &Job, spec: Option<&TimeSpec>, out: &Path, codec: video::Codec, scale: f32) -> Result<Outcome<Video>> {
    let (mut session, from, to) = job.range(spec)?;
    let opts = video::VideoOptions { from, to, codec, scale, background: job.eval.background, out: out.to_path_buf() };
    session.labels = false;
    let report = video::render(&mut session, job.eval, &opts)?;
    job.keep(&session);
    let result = Video { file: crate::slash(out), codec: codec.name(), from, to, frames: report.frames, frames_with_errors: report.errors };
    Ok(Outcome { range: Some((from, to)), errors: report.errors > 0, ..Outcome::new(result) })
}

// ─── perf ───────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct Perf {
    pub from: f64,
    pub to: f64,
    pub frames: usize,
    pub fps: f64,
    /// Бюджет кадра `1/fps`, мс.
    pub budget_ms: f64,
    /// Сборка kine: в debug замеры в разы медленнее release.
    pub build: &'static str,
    pub scale: f32,
    /// Запуск сцены: исполнение бандла и монтирование, мс.
    pub mount_ms: f64,
    /// По стадиям и `frame` — кадр целиком.
    pub stages: Vec<StageStats>,
    /// Кадров дольше бюджета.
    pub over_budget: usize,
    /// Самые долгие кадры.
    pub slowest: Vec<SlowFrame>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Serialize)]
pub struct StageStats {
    pub stage: &'static str,
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
}

#[derive(Debug, Serialize)]
pub struct SlowFrame {
    pub at: At,
    pub ms: f64,
}

fn ms(d: std::time::Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Миллисекунды до тысячных: замер не точнее.
fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

fn stats(stage: &'static str, values: &[f64]) -> StageStats {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let pick = |q: f64| v.get(((v.len() as f64 - 1.0) * q).round() as usize).copied().unwrap_or(0.0);
    let mean = if v.is_empty() { 0.0 } else { v.iter().sum::<f64>() / v.len() as f64 };
    StageStats { stage, mean_ms: r3(mean), p50_ms: r3(pick(0.5)), p95_ms: r3(pick(0.95)), max_ms: r3(pick(1.0)) }
}

/// Время кадра по стадиям на каждом кадре диапазона: script (команды и JS), motion, layout
/// (раскладка, текст, `must`), raster (рисование). p95 кадра больше `1/fps` — `perf.budget`.
pub fn perf(job: &Job, spec: Option<&TimeSpec>, scale: f32) -> Result<Outcome<Perf>> {
    let (resolved, from, to) = job.range(spec)?;
    // Границы диапазона разрешены; кадры меряются на свежей сессии, чтобы перемотка назад
    // не попала в первый кадр.
    drop(resolved);
    let start = std::time::Instant::now();
    let mut session = job.session()?;
    let mount_ms = r3(ms(start.elapsed()));
    session.labels = false;
    let opts = RenderOptions { scale, background: job.eval.background };
    let mut renderer = Renderer::default();
    let names = ["script", "motion", "layout", "raster", "frame"];
    let mut columns: [Vec<f64>; 5] = Default::default();
    let mut frames = Vec::new();
    for n in session.frames(from, to) {
        let t = session.frame_time(n);
        let (frame, times) = session.frame_timed(t, job.eval)?;
        let raster_start = std::time::Instant::now();
        renderer.render(&frame, &opts)?;
        let raster = ms(raster_start.elapsed());
        let row = [ms(times.script), ms(times.motion), ms(times.layout), raster];
        let total: f64 = row.iter().sum();
        for (col, v) in columns.iter_mut().zip(row.iter().chain([total].iter())) {
            col.push(*v);
        }
        frames.push((At { t, frame: n }, total));
    }
    job.keep(&session);
    let budget_ms = 1000.0 / session.fps;
    let stages: Vec<StageStats> = names.iter().zip(&columns).map(|(n, c)| stats(n, c)).collect();
    let over_budget = frames.iter().filter(|(_, t)| *t > budget_ms).count();
    frames.sort_by(|a, b| b.1.total_cmp(&a.1));
    let slowest = frames.iter().take(3).map(|(at, t)| SlowFrame { at: *at, ms: r3(*t) }).collect();
    let build = if cfg!(debug_assertions) { "debug" } else { "release" };
    let mut diagnostics = Vec::new();
    let total = &stages[4];
    if total.p95_ms > budget_ms {
        let worst = stages[..4].iter().max_by(|a, b| a.p95_ms.total_cmp(&b.p95_ms)).map_or("", |s| s.stage);
        let debug = if build == "debug" { "; замер debug-сборки — проверь на release (cargo build --release)" } else { "" };
        diagnostics.push(Diagnostic {
            level: Level::Warn,
            rule: "perf.budget".into(),
            node: "scene".into(),
            source: None,
            message: format!("кадр p95 {} мс при бюджете {} мс ({} fps)", total.p95_ms, r3(budget_ms), session.fps),
            hint: Some(format!("больше всего времени уходит на {worst}{debug}")),
            detail: None,
        });
    }
    let result = Perf {
        from,
        to,
        frames: columns[4].len(),
        fps: session.fps,
        budget_ms: r3(budget_ms),
        build,
        scale,
        mount_ms,
        stages,
        over_budget,
        slowest,
        diagnostics,
    };
    Ok(Outcome { range: Some((from, to)), ..Outcome::new(result) })
}

// ─── schema ─────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct Schema {
    pub stage: StageSchema,
    /// Данные сцены: `--props`, `--set`, `update` без компонента.
    pub props: Vec<PropInfo>,
    pub components: Vec<ComponentSchema>,
    /// Команды сценария (`*.cues.ts` и `--cue`).
    pub cues: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct StageSchema {
    pub size: [f32; 2],
    pub fps: f32,
}

#[derive(Debug, Serialize)]
pub struct ComponentSchema {
    #[serde(flatten)]
    pub info: ComponentInfo,
    /// Экземпляры в сцене.
    pub instances: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeline: Option<TimelineSchema>,
}

#[derive(Debug, Serialize)]
pub struct TimelineSchema {
    /// Маркеры по времени таймлайна.
    pub markers: Vec<(String, f64)>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub holds: Vec<String>,
    /// Цикл, в том числе неявный `loop..out`.
    #[serde(rename = "loop", skip_serializing_if = "Option::is_none")]
    pub repeat: Option<LoopDef>,
}

pub fn schema(job: &Job) -> Result<Outcome<Schema>> {
    let session = job.session()?;
    job.keep(&session);
    let scene = &session.scene;
    let stage = scene.stage.as_ref().context("в сцене нет stage")?;
    let components = scene
        .components
        .iter()
        .map(|c| {
            let mut instances: Vec<String> =
                scene.nodes.values().filter_map(|n| n.instance()).filter(|i| component_of(i) == c.name).map(String::from).collect();
            instances.sort();
            let timeline = scene.timeline(&c.name).map(|t| TimelineSchema {
                markers: t.markers_by_time().into_iter().map(|(n, v)| (n.to_string(), v)).collect(),
                holds: t.holds.clone(),
                repeat: t.repeat.clone(),
            });
            ComponentSchema { info: c.clone(), instances, timeline }
        })
        .collect();
    Ok(Outcome::new(Schema {
        stage: StageSchema { size: stage.size, fps: stage.fps },
        props: scene.props.clone(),
        components,
        cues: session.cues().iter().map(|c| c.label()).collect(),
    }))
}

/// Подложка для `--bg`.
pub fn parse_hex(s: &str) -> Result<Rgba> {
    Rgba::from_hex(s).with_context(|| format!("--bg: ожидается #RRGGBB, получено {s:?}"))
}
