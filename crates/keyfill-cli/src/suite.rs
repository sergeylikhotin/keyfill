//! `keyfill test`: случаи `*.keyfill.test.ts`, проверка `clean` и снапшоты дампа и пикселей.
//!
//! Файл тестов собирается как сцена. Сначала бандл отдаёт список случаев (`EntryMode::Tests`),
//! потом каждый случай монтирует свою сцену с пропсами (`EntryMode::Case`) — дальше это обычный
//! `Job`, и моменты проверяются теми же операциями, что `dump`, `render` и `check`.
//!
//! Снапшоты лежат рядом с тестом: `__snapshots__/<файл>/<случай>@<t>.dump.txt|.png`. Нет файла —
//! он пишется; расходится — случай падает, а `--update` перезаписывает. Разница дампа — по узлам
//! (появился, пропал, изменился), пикселей — число отличий, их рамка и маска в `.keyfill/test`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use keyfill_core::diag::Level;
use keyfill_core::dump;
use keyfill_core::{EvalOptions, Mode};
use keyfill_js::{EntryMode, JsHost, RunOptions};
use keyfill_render::{Image, Png, RenderOptions, Renderer};
use serde::{Deserialize, Serialize};

use crate::ops::{self, At, Issue, Job, Outcome};
use crate::output::{ErrorInfo, classify};
use crate::time::{self, TimeSpec};
use crate::{Bundle, slash};

pub const SUFFIX: &str = ".keyfill.test.ts";
/// Каталоги, в которых тесты не ищутся.
const SKIP: &[&str] = &["node_modules", "target", "out", "dist", "__snapshots__"];
/// Сколько строк разницы дампа показывать.
const DIFF_LIMIT: usize = 30;

pub struct SuiteOptions {
    /// Файлы и каталоги; пусто — каталог запуска.
    pub paths: Vec<PathBuf>,
    /// Только случаи, в имени которых есть эта строка.
    pub name: Option<String>,
    pub update: bool,
    /// Сид и `Date.now` для всех случаев.
    pub run: RunOptions,
}

// ─── Отчёт ──────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct Report {
    pub files: usize,
    pub counts: TestCounts,
    pub cases: Vec<CaseReport>,
}

#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct TestCounts {
    pub passed: usize,
    pub failed: usize,
    /// Новых снапшотов записано.
    pub written: usize,
    /// Снапшотов перезаписано `--update`.
    pub updated: usize,
}

#[derive(Debug, Serialize)]
pub struct CaseReport {
    pub file: String,
    /// Имя случая; пусто — ошибка всего файла (сборка, исключение при загрузке).
    pub name: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorInfo>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub moments: Vec<MomentReport>,
}

#[derive(Debug, Serialize)]
pub struct MomentReport {
    /// Момент, как он записан в тесте.
    pub t: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<At>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<(f64, f64)>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorInfo>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<Check>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "check", rename_all = "lowercase")]
pub enum Check {
    /// Ошибки диагностики в момент или на кадрах диапазона.
    Clean { ok: bool, #[serde(skip_serializing_if = "Vec::is_empty")] issues: Vec<Issue> },
    Dump {
        status: SnapStatus,
        file: String,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        diff: Vec<DiffEntry>,
        /// Строк разницы сверх показанных.
        #[serde(skip_serializing_if = "is_zero")]
        more: usize,
    },
    Png {
        status: SnapStatus,
        file: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pixels: Option<PixelDiff>,
    },
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

impl Check {
    fn ok(&self) -> bool {
        match self {
            Check::Clean { ok, .. } => *ok,
            Check::Dump { status, .. } | Check::Png { status, .. } => *status != SnapStatus::Changed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SnapStatus {
    /// Совпал со снапшотом.
    Same,
    /// Снапшота не было — записан.
    Written,
    /// Расходился и перезаписан `--update`.
    Updated,
    /// Расходится: случай упал.
    Changed,
}

/// Строка разницы дампа: `+` — появилось, `-` — пропало, `~` — узел изменился.
#[derive(Debug, Serialize)]
pub struct DiffEntry {
    pub op: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PixelDiff {
    /// Пикселей, у которых канал отличается больше допуска.
    pub differing: usize,
    pub total: usize,
    /// Рамка отличий в пикселях: x, y, w, h.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<[u32; 4]>,
    /// Размеры снапшота и нового кадра, если они разные.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<([u32; 2], [u32; 2])>,
    /// Новый кадр и маска отличий (пурпурным).
    pub actual: String,
    pub mask: String,
}

impl Report {
    /// Файлы тестов, у которых снапшот разошёлся: для подсказки `keyfill test … --update`.
    pub fn failing_snapshots(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .cases
            .iter()
            .filter(|c| c.moments.iter().flat_map(|m| &m.checks).any(|k| matches!(k, Check::Dump { status: SnapStatus::Changed, .. } | Check::Png { status: SnapStatus::Changed, .. })))
            .map(|c| c.file.clone())
            .collect();
        out.dedup();
        out
    }

    /// Правило первой ошибки `clean` — для подсказки `keyfill explain`.
    pub fn first_error_rule(&self) -> Option<String> {
        self.cases.iter().flat_map(|c| &c.moments).flat_map(|m| &m.checks).find_map(|k| match k {
            Check::Clean { issues, .. } => issues.first().map(|i| i.diagnostic.rule.clone()),
            _ => None,
        })
    }
}

// ─── Случаи из бандла ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseSpec {
    name: String,
    #[serde(default)]
    t: Vec<serde_json::Value>,
    mode: Option<String>,
    bg: Option<String>,
    scale: Option<f32>,
    #[serde(default)]
    expect: Expect,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Expect {
    clean: Option<bool>,
    snapshot: Option<Snapshots>,
    tolerance: Option<Tolerance>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Snapshots {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tolerance {
    #[serde(default = "Tolerance::channel")]
    pub channel: u8,
    #[serde(default = "Tolerance::pixels")]
    pub pixels: f64,
}

impl Tolerance {
    fn channel() -> u8 {
        2
    }
    fn pixels() -> f64 {
        0.001
    }
}

impl Default for Tolerance {
    fn default() -> Self {
        Tolerance { channel: Tolerance::channel(), pixels: Tolerance::pixels() }
    }
}

impl CaseSpec {
    fn snapshots(&self) -> Result<(bool, bool)> {
        let kinds: Vec<&str> = match &self.expect.snapshot {
            None => Vec::new(),
            Some(Snapshots::One(s)) => vec![s.as_str()],
            Some(Snapshots::Many(v)) => v.iter().map(String::as_str).collect(),
        };
        if let Some(bad) = kinds.iter().find(|k| !["dump", "png"].contains(k)) {
            return Err(crate::usage(format!("expect.snapshot: dump или png, получено {bad}")));
        }
        Ok((kinds.contains(&"dump"), kinds.contains(&"png")))
    }

    fn eval(&self) -> Result<EvalOptions> {
        let mode = match self.mode.as_deref() {
            None | Some("dev") => Mode::Dev,
            Some("air") => Mode::Air,
            Some(m) => return Err(crate::usage(format!("mode: dev или air, получено {m}"))),
        };
        Ok(EvalOptions { mode, background: self.bg.as_deref().map(ops::parse_hex).transpose()? })
    }

    /// Моменты как строки грамматики времени; по умолчанию `settled`.
    fn times(&self) -> Result<Vec<String>> {
        if self.t.is_empty() {
            return Ok(vec!["settled".into()]);
        }
        self.t
            .iter()
            .map(|v| match v {
                serde_json::Value::Number(n) => Ok(n.to_string()),
                serde_json::Value::String(s) => Ok(s.clone()),
                other => Err(crate::usage(format!("t: ожидается число или строка времени, получено {other}"))),
            })
            .collect()
    }
}

/// Случаи файла тестов: бандл без монтирования отдаёт их описания.
fn list(bundle: &Bundle, run: &RunOptions) -> Result<Vec<CaseSpec>> {
    let opts = RunOptions { entry: EntryMode::Tests, ..run.clone() };
    let (_, out) = JsHost::start(&bundle.code, &opts).map_err(|e| crate::session::map_js(bundle.map.as_ref(), e))?;
    let items = out
        .batches
        .iter()
        .filter_map(|b| serde_json::from_str::<Vec<serde_json::Value>>(b).ok())
        .flatten()
        .find(|c| c["op"] == "tests")
        .map(|c| c["items"].clone())
        .context("бандл тестов не отдал список случаев")?;
    let items: Vec<serde_json::Value> = serde_json::from_value(items)?;
    items
        .into_iter()
        .map(|v| {
            let name = v["name"].as_str().unwrap_or("?").to_string();
            serde_json::from_value(v).map_err(|e| crate::usage(format!("test \"{name}\": {e}")))
        })
        .collect()
}

// ─── Поиск файлов ───────────────────────────────────────────────────────────

pub fn discover(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let roots = if paths.is_empty() { vec![PathBuf::from(".")] } else { paths.to_vec() };
    let mut out = Vec::new();
    for root in &roots {
        if root.is_file() {
            out.push(root.clone());
        } else if root.is_dir() {
            walk(root, &mut out);
        } else {
            return Err(crate::usage(format!("{} не найден", slash(root))));
        }
    }
    out.sort();
    out.dedup();
    if out.is_empty() {
        let at: Vec<String> = roots.iter().map(|r| slash(r)).collect();
        return Err(crate::usage(format!("нет файлов *{SUFFIX} в {}; пример — keyfill docs test", at.join(", "))));
    }
    Ok(out)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            if !name.starts_with('.') && !SKIP.contains(&name.as_str()) {
                walk(&p, out);
            }
        } else if name.ends_with(SUFFIX) {
            out.push(p.strip_prefix(".").map(Path::to_path_buf).unwrap_or(p));
        }
    }
}

// ─── Запуск ─────────────────────────────────────────────────────────────────

pub fn run(opts: &SuiteOptions) -> Result<Outcome<Report>> {
    let files = discover(&opts.paths)?;
    let mut cases = Vec::new();
    for file in &files {
        run_file(file, opts, &mut cases);
    }
    let mut counts = TestCounts::default();
    for c in &cases {
        if c.ok {
            counts.passed += 1;
        } else {
            counts.failed += 1;
        }
        for check in c.moments.iter().flat_map(|m| &m.checks) {
            match check {
                Check::Dump { status: SnapStatus::Written, .. } | Check::Png { status: SnapStatus::Written, .. } => counts.written += 1,
                Check::Dump { status: SnapStatus::Updated, .. } | Check::Png { status: SnapStatus::Updated, .. } => counts.updated += 1,
                _ => {}
            }
        }
    }
    let report = Report { files: files.len(), counts, cases };
    Ok(Outcome { errors: counts.failed > 0, ..Outcome::new(report) })
}

fn file_error(file: &str, e: &anyhow::Error) -> CaseReport {
    CaseReport { file: file.into(), name: String::new(), ok: false, error: Some(classify(e).1), moments: Vec::new() }
}

fn run_file(file: &Path, opts: &SuiteOptions, out: &mut Vec<CaseReport>) {
    let shown = slash(file);
    let bundle = match crate::bundle(file) {
        Ok(b) => b,
        Err(e) => return out.push(file_error(&shown, &e)),
    };
    let specs = match list(&bundle, &opts.run) {
        Ok(s) => s,
        Err(e) => return out.push(file_error(&shown, &e)),
    };
    let dir = snapshot_dir(file);
    let mut produced = BTreeSet::new();
    let mut slugs = BTreeSet::new();
    for (i, spec) in specs.iter().enumerate() {
        if opts.name.as_deref().is_some_and(|n| !spec.name.contains(n)) {
            continue;
        }
        let slug = slug(&spec.name);
        if !slugs.insert(slug.clone()) {
            let e = crate::usage(format!("имя случая «{}» совпадает с другим после замены символов ({slug}); переименуй", spec.name));
            out.push(CaseReport { name: spec.name.clone(), ..file_error(&shown, &e) });
            continue;
        }
        let ctx = CaseCtx { file, bundle: &bundle, index: i, spec, dir: &dir, slug: &slug, update: opts.update, run: &opts.run };
        let report = match run_case(&ctx, &mut produced) {
            Ok(moments) => CaseReport { file: shown.clone(), name: spec.name.clone(), ok: moments.iter().all(|m| m.ok), error: None, moments },
            Err(e) => CaseReport { name: spec.name.clone(), ..file_error(&shown, &e) },
        };
        out.push(report);
    }
    // Снапшоты случаев, которых больше нет: при полном прогоне с --update они удаляются.
    if opts.update && opts.name.is_none() {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            if !produced.contains(&e.path()) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

struct CaseCtx<'a> {
    file: &'a Path,
    bundle: &'a Bundle,
    index: usize,
    spec: &'a CaseSpec,
    dir: &'a Path,
    slug: &'a str,
    update: bool,
    run: &'a RunOptions,
}

fn run_case(c: &CaseCtx, produced: &mut BTreeSet<PathBuf>) -> Result<Vec<MomentReport>> {
    let eval = c.spec.eval()?;
    let (dump_snap, png_snap) = c.spec.snapshots()?;
    let clean = c.spec.expect.clean.unwrap_or(true);
    let tolerance = c.spec.expect.tolerance.unwrap_or_default();
    let scale = c.spec.scale.unwrap_or(1.0);
    if scale.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return Err(crate::usage(format!("scale: ожидается число больше 0, получено {scale}")));
    }
    let run = RunOptions { entry: EntryMode::Case(c.index), ..c.run.clone() };
    let job = Job::with_bundle(c.file, c.bundle.clone(), run, Vec::new(), eval);
    let mut moments = Vec::new();
    for t in c.spec.times()? {
        let report = moment(&job, c, &t, clean, (dump_snap, png_snap), tolerance, scale, produced);
        moments.push(report.unwrap_or_else(|e| {
            MomentReport { t: t.clone(), at: None, range: None, ok: false, error: Some(classify(&e).1), checks: Vec::new() }
        }));
        // Ошибка данных сцены или JS повторится в каждом моменте: дальше не продолжаем. Ошибка
        // самого момента (неизвестный маркер) — только его.
        if moments.last().and_then(|m| m.error.as_ref()).is_some_and(|e| e.kind != "usage" || !e.diagnostics.is_empty()) {
            break;
        }
    }
    Ok(moments)
}

#[allow(clippy::too_many_arguments)]
fn moment(
    job: &Job,
    c: &CaseCtx,
    t: &str,
    clean: bool,
    (dump_snap, png_snap): (bool, bool),
    tolerance: Tolerance,
    scale: f32,
    produced: &mut BTreeSet<PathBuf>,
) -> Result<MomentReport> {
    let spec = time::parse(t).map_err(|e| crate::usage(format!("t {t:?}: {e:#}")))?;
    let mut checks = Vec::new();
    let report = match &spec {
        // Диапазон проверяется только на clean: снапшоты снимаются в моменты того же случая.
        TimeSpec::Range(..) => {
            let check = ops::check(job, Some(&spec))?;
            if clean {
                let issues: Vec<Issue> = check.result.issues.into_iter().filter(|i| i.diagnostic.level == Level::Error).collect();
                checks.push(Check::Clean { ok: issues.is_empty(), issues });
            }
            MomentReport { t: t.into(), at: None, range: check.range, ok: true, error: None, checks }
        }
        TimeSpec::Moment(m) => {
            let (session, frame, at) = ops::frame(job, Some(m))?;
            if clean {
                let issues: Vec<Issue> = frame
                    .diagnostics
                    .0
                    .iter()
                    .filter(|d| d.level == Level::Error)
                    .map(|d| Issue { diagnostic: d.clone(), first: at, last: at, frames: 1 })
                    .collect();
                checks.push(Check::Clean { ok: issues.is_empty(), issues });
            }
            // Имя момента может содержать точку (`out+0.1`): расширение дописывается, а не заменяется.
            let base = format!("{}@{}", c.slug, slug(t));
            if dump_snap {
                let path = c.dir.join(format!("{base}.dump.txt"));
                let text = snapshot_text(&dump::text(&dump::build(&session.scene, &frame)));
                checks.push(dump_check(&path, &text, c.update)?);
                produced.insert(path);
            }
            if png_snap {
                let path = c.dir.join(format!("{base}.png"));
                let mut renderer = Renderer::default();
                let pixmap = renderer.render(&frame, &RenderOptions { scale, background: job.eval.background })?;
                checks.push(png_check(&path, &keyfill_render::image(pixmap), tolerance, c.update)?);
                produced.insert(path);
            }
            MomentReport { t: t.into(), at: Some(at), range: None, ok: true, error: None, checks }
        }
    };
    Ok(MomentReport { ok: report.checks.iter().all(Check::ok), ..report })
}

// ─── Снапшоты ───────────────────────────────────────────────────────────────

/// `examples/x/lower.keyfill.test.ts` → `examples/x/__snapshots__/lower`.
fn snapshot_dir(file: &Path) -> PathBuf {
    let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let stem = name.strip_suffix(SUFFIX).unwrap_or(&name).to_string();
    file.parent().unwrap_or(Path::new(".")).join("__snapshots__").join(stem)
}

/// Имя случая или момента в имени файла: буквы и цифры (в том числе кириллица), `+ - _ .`;
/// остальное — `-`.
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for ch in s.trim().chars() {
        let c = if ch.is_alphanumeric() || "+-_".contains(ch) { ch } else if ch == '.' { '.' } else { '-' };
        if !(c == '-' && out.ends_with('-')) {
            out.push(c);
        }
    }
    let out = out.trim_matches('-').replace("..", "-");
    if out.is_empty() { "case".into() } else { out }
}

/// Дамп для снапшота без мест в исходнике (`    at file:line:col`): снапшот закрепляет
/// графику, а не номера строк, и вставка строки кода выше узла его не меняет.
fn snapshot_text(dump: &str) -> String {
    dump.lines().filter(|l| !l.starts_with("    at ")).map(|l| format!("{l}\n")).collect()
}

fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("не удалось создать {}", dir.display()))?;
    }
    std::fs::write(path, bytes).with_context(|| format!("не удалось записать {}", path.display()))
}

fn dump_check(path: &Path, actual: &str, update: bool) -> Result<Check> {
    let file = slash(path);
    let old = match std::fs::read_to_string(path) {
        Ok(old) => old.replace("\r\n", "\n"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            write(path, actual.as_bytes())?;
            return Ok(Check::Dump { status: SnapStatus::Written, file, diff: Vec::new(), more: 0 });
        }
        Err(e) => return Err(e).with_context(|| format!("не удалось прочитать {file}")),
    };
    if old == actual {
        return Ok(Check::Dump { status: SnapStatus::Same, file, diff: Vec::new(), more: 0 });
    }
    if update {
        write(path, actual.as_bytes())?;
        return Ok(Check::Dump { status: SnapStatus::Updated, file, diff: Vec::new(), more: 0 });
    }
    let mut diff = diff_dumps(&old, actual);
    let more = diff.len().saturating_sub(DIFF_LIMIT);
    diff.truncate(DIFF_LIMIT);
    Ok(Check::Dump { status: SnapStatus::Changed, file, diff, more })
}

/// Строка узла дампа: `├─ LowerThird#1/title  text  (…)` → адрес и остальное.
fn node_line(line: &str) -> Option<(&str, &str)> {
    let i = line.find("├─ ").or_else(|| line.find("└─ "))?;
    let content = &line[i + "├─ ".len()..];
    Some(content.split_once("  ").map_or((content, ""), |(p, rest)| (p, rest.trim())))
}

/// Разница двух текстовых дампов по узлам: узлы сопоставляются по адресу, а не по позиции,
/// поэтому вставка узла не «сдвигает» остальные. Прочие строки (кадр, события, диагностики)
/// сравниваются как множества.
pub fn diff_dumps(old: &str, new: &str) -> Vec<DiffEntry> {
    let old_nodes: Vec<(&str, &str)> = old.lines().filter_map(node_line).collect();
    let new_nodes: Vec<(&str, &str)> = new.lines().filter_map(node_line).collect();
    let mut old_other: Vec<&str> = old.lines().filter(|l| node_line(l).is_none()).collect();
    let mut out = Vec::new();
    for line in new.lines() {
        match node_line(line) {
            Some((path, body)) => match old_nodes.iter().find(|(p, _)| *p == path) {
                Some((_, was)) if *was == body => {}
                Some((_, was)) => out.push(DiffEntry { op: "~", node: Some(path.into()), old: Some((*was).into()), new: Some(body.into()) }),
                None => out.push(DiffEntry { op: "+", node: Some(path.into()), old: None, new: Some(body.into()) }),
            },
            None => match old_other.iter().position(|l| *l == line) {
                Some(i) => {
                    old_other.remove(i);
                }
                None => out.push(DiffEntry { op: "+", node: None, old: None, new: Some(line.into()) }),
            },
        }
    }
    for (path, body) in &old_nodes {
        if !new_nodes.iter().any(|(p, _)| p == path) {
            out.push(DiffEntry { op: "-", node: Some((*path).into()), old: Some((*body).into()), new: None });
        }
    }
    for line in old_other {
        out.push(DiffEntry { op: "-", node: None, old: Some(line.into()), new: None });
    }
    out
}

fn png_check(path: &Path, actual: &Image, tol: Tolerance, update: bool) -> Result<Check> {
    let file = slash(path);
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            write(path, &keyfill_render::encode_png(actual, Png::Small)?)?;
            return Ok(Check::Png { status: SnapStatus::Written, file, pixels: None });
        }
        Err(e) => return Err(e).with_context(|| format!("не удалось прочитать {file}")),
    };
    let reference = keyfill_render::decode_png(&bytes).with_context(|| format!("{file}: не PNG"))?;
    let (differing, bbox, mask) = compare(&reference, actual, tol.channel);
    let total = actual.rgba.len() / 4;
    let same_size = reference.width == actual.width && reference.height == actual.height;
    if same_size && (differing as f64) <= tol.pixels * total as f64 {
        return Ok(Check::Png { status: SnapStatus::Same, file, pixels: None });
    }
    if update {
        write(path, &keyfill_render::encode_png(actual, Png::Small)?)?;
        return Ok(Check::Png { status: SnapStatus::Updated, file, pixels: None });
    }
    // Новый кадр и маска — в .keyfill/test с тем же путём, что у снапшота.
    let rel = std::path::absolute(path)?;
    let rel = rel.strip_prefix(std::env::current_dir()?).map(Path::to_path_buf).unwrap_or_else(|_| PathBuf::from(path.file_name().unwrap_or_default()));
    let base = Path::new(".keyfill").join("test").join(rel);
    let (actual_path, mask_path) = (base.with_extension("actual.png"), base.with_extension("diff.png"));
    write(&actual_path, &keyfill_render::encode_png(actual, Png::Fast)?)?;
    write(&mask_path, &keyfill_render::encode_png(&mask, Png::Fast)?)?;
    let pixels = PixelDiff {
        differing,
        total,
        bbox,
        size: (!same_size).then_some(([reference.width, reference.height], [actual.width, actual.height])),
        actual: slash(&actual_path),
        mask: slash(&mask_path),
    };
    Ok(Check::Png { status: SnapStatus::Changed, file, pixels: Some(pixels) })
}

/// Сравнение по пикселям: сколько отличается больше допуска, рамка отличий и маска —
/// приглушённый новый кадр, отличия пурпурным. Разный размер — отличаются все пиксели.
fn compare(a: &Image, b: &Image, channel: u8) -> (usize, Option<[u32; 4]>, Image) {
    let mut mask = Image { width: b.width, height: b.height, rgba: Vec::with_capacity(b.rgba.len()) };
    let same_size = a.width == b.width && a.height == b.height;
    let (mut n, mut x0, mut y0, mut x1, mut y1) = (0, u32::MAX, u32::MAX, 0, 0);
    let reference = a.rgba.as_chunks::<4>().0;
    for (i, p) in b.rgba.as_chunks::<4>().0.iter().enumerate() {
        let q = if same_size { reference[i] } else { [0; 4] };
        let differs = !same_size || p.iter().zip(q).any(|(x, y)| x.abs_diff(y) > channel);
        if differs {
            n += 1;
            let (x, y) = (i as u32 % b.width, i as u32 / b.width);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            mask.rgba.extend_from_slice(&[255, 0, 255, 255]);
        } else {
            // Яркость кадра на трети, поверх тёмного: видно, где что, а отличия выделяются.
            let lum = (u32::from(p[0]) * 2 + u32::from(p[1]) * 5 + u32::from(p[2])) / 8;
            let v = (24 + lum * u32::from(p[3]) / 255 / 3) as u8;
            mask.rgba.extend_from_slice(&[v, v, v, 255]);
        }
    }
    let bbox = (n > 0).then(|| [x0, y0, x1 - x0 + 1, y1 - y0 + 1]);
    (n, bbox, mask)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_file_names() {
        assert_eq!(slug("длинное имя"), "длинное-имя");
        assert_eq!(slug("out+0.1"), "out+0.1");
        assert_eq!(slug("0..4"), "0-4");
        assert_eq!(slug("a/b: c?"), "a-b-c");
        assert_eq!(slug("   "), "case");
    }

    #[test]
    fn dump_diff_matches_nodes_by_path() {
        let old = "time: t=1s\n└─ A#1/panel  box  (0,0 10×10)\n   ├─ A#1/title  text  (0,0 5×5)  \"Анна\"\n   └─ A#1/dot  box  (0,0 1×1)\nerror must.fit A#1/title: x\n";
        let new = "time: t=1s\n└─ A#1/panel  box  (0,0 10×10)\n   ├─ A#1/badge  box  (0,0 2×2)\n   ├─ A#1/title  text  (0,0 6×5)  \"Иван\"\n";
        let d = diff_dumps(old, new);
        let ops: Vec<(&str, Option<&str>)> = d.iter().map(|e| (e.op, e.node.as_deref())).collect();
        assert_eq!(ops, [("+", Some("A#1/badge")), ("~", Some("A#1/title")), ("-", Some("A#1/dot")), ("-", None)]);
        assert_eq!(d[1].new.as_deref(), Some("text  (0,0 6×5)  \"Иван\""));
    }

    #[test]
    fn pixel_compare_counts_beyond_tolerance() {
        let a = Image { width: 2, height: 1, rgba: vec![10, 10, 10, 255, 0, 0, 0, 255] };
        let b = Image { width: 2, height: 1, rgba: vec![12, 10, 10, 255, 0, 9, 0, 255] };
        let (n, bbox, mask) = compare(&a, &b, 2);
        assert_eq!((n, bbox), (1, Some([1, 0, 1, 1])));
        assert_eq!(&mask.rgba[4..], &[255, 0, 255, 255]);
        let c = Image { width: 1, height: 1, rgba: vec![0; 4] };
        assert_eq!(compare(&a, &c, 2).0, 1, "другой размер — отличаются все пиксели");
    }
}
