//! Операции агента: render, dump, inspect, check, filmstrip, video, schema.
//!
//! Один слой для CLI и (позже) `kine serve` и MCP. Каждая операция возвращает структуру;
//! текст для терминала строится из неё же (`output.rs`), поэтому `--json` и текст не расходятся.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use kine_core::diag::{Diagnostic, Level};
use kine_core::dump::{self, Dump, Inspect};
use kine_core::look::Rgba;
use kine_core::scene::{ComponentInfo, PropInfo};
use kine_core::{EvalOptions, Frame};
use kine_js::{LogLine, RunOptions};
use kine_render::RenderOptions;
use serde::Serialize;
use serde_json::Value;

use crate::session::CueArg;
use crate::time::{self, Moment, Offset, TimeSpec};
use crate::{Bundle, Failure, Session, filmstrip, video};

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

impl Job {
    pub fn new(entry: &Path, run: RunOptions, cues: Vec<CueArg>, eval: EvalOptions) -> Result<Job> {
        let bundle = crate::bundle(entry)?;
        Ok(Job { entry: entry.to_path_buf(), bundle, run, cues, eval, logs: Default::default() })
    }

    pub fn session(&self) -> Result<Session> {
        Session::open(&self.bundle, &self.run, &self.cues)
    }

    /// Запоминает console.* сессии, которой операция построила результат.
    fn keep(&self, session: &Session) {
        *self.logs.borrow_mut() = session.logs.clone();
    }

    fn at(&self, session: &Session, t: f64) -> At {
        At { t, frame: (t * session.fps).round() as u64 }
    }

    /// Момент из `--t`; без него — кадр «в эфире» (`settled`). `settled` и маркеры продвигают
    /// сессию, поэтому момент разрешается на пробной сессии.
    pub fn moment(&self, spec: Option<&TimeSpec>) -> Result<f64> {
        let moment = match spec {
            Some(TimeSpec::Moment(m)) => m.clone(),
            Some(TimeSpec::Range(..)) => bail!("--t: здесь нужен момент, а не диапазон"),
            None => Moment::Settled(Offset::Seconds(0.0)),
        };
        let mut probe = self.session()?;
        time::resolve(&moment, &mut probe).map_err(crate::usage)
    }

    /// Диапазон из `--t`: `a..b`, имя сегмента (`in`, `out`, маркер таймлайна) или один
    /// момент; без `--t` — весь сценарий.
    pub fn range(&self, spec: Option<&TimeSpec>) -> Result<(f64, f64)> {
        let mut probe = self.session()?;
        let (from, to) = match spec {
            Some(TimeSpec::Range(a, b)) => {
                (time::resolve(a, &mut probe).map_err(crate::usage)?, time::resolve(b, &mut probe).map_err(crate::usage)?)
            }
            Some(TimeSpec::Moment(Moment::Marker(name, Offset::Seconds(s)))) if *s == 0.0 => {
                time::segment(name, &mut probe).map_err(crate::usage)?
            }
            Some(TimeSpec::Moment(m)) => {
                let t = time::resolve(m, &mut probe).map_err(crate::usage)?;
                (t, t)
            }
            None => (0.0, probe.end_time()?),
        };
        if to < from {
            return Err(anyhow::anyhow!("--t: конец диапазона {to} раньше начала {from}").context(Failure::Usage));
        }
        Ok((from, to))
    }
}

/// Кадр в момент `t` на свежей сессии.
pub fn frame(job: &Job, t: f64) -> Result<(Session, Frame, At)> {
    let mut session = job.session()?;
    let t = session.nearest_frame(t);
    let frame = session.frame_at(t, job.eval)?;
    let at = job.at(&session, t);
    job.keep(&session);
    Ok((session, frame, at))
}

pub fn has_errors(diags: &[Diagnostic]) -> bool {
    diags.iter().any(|d| d.level == Level::Error)
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

#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct Counts {
    pub error: usize,
    pub warn: usize,
    pub info: usize,
}

impl Counts {
    pub fn of<'a>(diags: impl IntoIterator<Item = &'a Diagnostic>) -> Counts {
        let mut c = Counts::default();
        for d in diags {
            match d.level {
                Level::Error => c.error += 1,
                Level::Warn => c.warn += 1,
                Level::Info => c.info += 1,
            }
        }
        c
    }
}

pub fn render(job: &Job, t: f64, out: &Path, scale: f32, with_dump: bool) -> Result<Rendered> {
    let (session, frame, at) = frame(job, t)?;
    let opts = RenderOptions { scale, background: job.eval.background };
    let png = kine_render::render_png(&frame, &opts)?;
    std::fs::write(out, png).with_context(|| format!("не удалось записать {}", out.display()))?;
    let dump = dump::build(&session.scene, &frame);
    Ok(Rendered {
        file: crate::slash(out),
        size: [(frame.size[0] * scale).round() as u32, (frame.size[1] * scale).round() as u32],
        at,
        counts: Counts::of(&dump.diagnostics),
        diagnostics: dump.diagnostics.clone(),
        dump: with_dump.then_some(dump),
    })
}

// ─── dump и inspect ─────────────────────────────────────────────────────────

pub fn dump(job: &Job, t: f64, path: Option<&str>, depth: Option<usize>) -> Result<(Dump, At)> {
    let (session, frame, at) = frame(job, t)?;
    let mut d = dump::build(&session.scene, &frame);
    d.filter(path, depth).map_err(|e| anyhow::anyhow!(e).context(Failure::Usage))?;
    Ok((d, at))
}

pub fn inspect(job: &Job, t: f64, path: &str) -> Result<(Inspect, At)> {
    let (session, frame, at) = frame(job, t)?;
    match dump::inspect_build(&session.scene, &frame, path) {
        Some(i) => Ok((i, at)),
        None => {
            let known = dump::build(&session.scene, &frame).paths();
            let near: Vec<&String> = known.iter().filter(|p| p.contains(path) || path.contains(p.as_str())).take(5).collect();
            let hint = if near.is_empty() { "адреса узлов есть в `kine dump`".to_string() } else { format!("похожие: {}", near.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")) };
            Err(anyhow::anyhow!("узел {path} не найден; {hint}").context(Failure::Usage))
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
pub fn check(job: &Job, from: f64, to: f64) -> Result<Check> {
    let mut session = job.session()?;
    let fps = session.fps;
    let (first, last) = ((from * fps).round() as u64, (to * fps).round() as u64);
    let mut issues: Vec<Issue> = Vec::new();
    let mut index: BTreeMap<(Level, String, String, Option<String>), usize> = BTreeMap::new();
    for n in first..=last {
        let t = n as f64 / fps;
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
    Ok(Check { from, to, frames: (last - first + 1) as usize, counts: Counts::of(issues.iter().map(|i| &i.diagnostic)), issues })
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

pub fn filmstrip(job: &Job, from: f64, to: f64, out: &Path, o: &FilmstripOptions) -> Result<Filmstrip> {
    let mut session = job.session()?;
    let fopts = filmstrip::Options { from, to, count: o.count, cols: o.cols, tile: o.tile, background: job.eval.background };
    let times = filmstrip::times(&session, &fopts);
    let strip = filmstrip::build(&mut session, &times, job.eval, &fopts)?;
    let png = kine_render::execute_png(&strip.display, &RenderOptions { scale: 1.0, background: None })?;
    std::fs::write(out, png).with_context(|| format!("не удалось записать {}", out.display()))?;
    let counts = Counts::of(strip.frames.iter().flat_map(|f| &f.diagnostics.0));
    job.keep(&session);
    Ok(Filmstrip {
        file: crate::slash(out),
        frames: times.iter().map(|&t| job.at(&session, t)).collect(),
        counts,
        dumps: if o.dumps { strip.dumps } else { Vec::new() },
    })
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

pub fn video(job: &Job, from: f64, to: f64, out: &Path, codec: video::Codec, scale: f32) -> Result<Video> {
    let opts = video::VideoOptions { from, to, codec, scale, background: job.eval.background, out: out.to_path_buf() };
    let mut session = job.session()?;
    let report = video::render(&mut session, job.eval, &opts)?;
    job.keep(&session);
    Ok(Video { file: crate::slash(out), codec: codec.name(), from, to, frames: report.frames, frames_with_errors: report.errors })
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
    #[serde(rename = "loop", skip_serializing_if = "Option::is_none")]
    pub repeat: Option<Value>,
}

/// Маркеры таймлайна компонента, по возрастанию времени.
pub fn timeline_markers(def: &Value) -> Vec<(String, f64)> {
    let mut out: Vec<(String, f64)> = def
        .get("markers")
        .and_then(Value::as_object)
        .map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_f64()?))).collect())
        .unwrap_or_default();
    out.sort_by(|a, b| a.1.total_cmp(&b.1));
    out
}

pub fn schema(job: &Job) -> Result<Schema> {
    let session = job.session()?;
    job.keep(&session);
    let scene = &session.scene;
    let stage = scene.stage.as_ref().context("в сцене нет stage")?;
    let components = scene
        .components
        .iter()
        .map(|c| {
            let mut instances: Vec<String> = scene.nodes.values().filter_map(|n| n.instance()).filter(|i| i.split('#').next() == Some(c.name.as_str())).map(String::from).collect();
            instances.sort();
            let timeline = scene.timelines.iter().find(|t| t.get("timeline").and_then(Value::as_str) == Some(c.name.as_str())).map(|t| TimelineSchema {
                markers: timeline_markers(t),
                holds: t.get("holds").and_then(Value::as_array).map(|h| h.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
                repeat: t.get("loop").cloned(),
            });
            ComponentSchema { info: c.clone(), instances, timeline }
        })
        .collect();
    Ok(Schema {
        stage: StageSchema { size: stage.size, fps: stage.fps },
        props: scene.props.clone(),
        components,
        cues: session.cues().iter().map(|c| c.label()).collect(),
    })
}

/// Подложка для `--bg`.
pub fn parse_hex(s: &str) -> Result<Rgba> {
    let h = s.trim_start_matches('#');
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("--bg: ожидается #RRGGBB, получено {s:?}");
    }
    let n = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(0);
    let json = serde_json::json!({ "r": n(0), "g": n(2), "b": n(4) });
    kine_core::look::parse_color(&json).map_err(anyhow::Error::msg)
}
