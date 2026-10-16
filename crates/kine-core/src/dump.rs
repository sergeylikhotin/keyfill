//! Представления кадра для агента: дамп сцены и inspect одного узла.
//!
//! Сначала строится структура (`Dump`, `Inspect`), из неё — и JSON (`--json`), и текст.
//! Поэтому текстовый и машинный вывод не расходятся.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Serialize;
use serde_json::Value;

use crate::diag::{Diagnostic, Level};
use crate::layout::{Frame, FrameNode, Rect};
use crate::look::{Look, num};
use crate::motion::RelMotion;
use crate::scene::{Kind, Scene};

/// Прямоугольник `[x, y, w, h]`.
pub type Box4 = [f32; 4];

fn b4(r: &Rect) -> Box4 {
    [r.x, r.y, r.w, r.h]
}

fn rect(r: &Box4) -> String {
    format!("({})", plain_rect(r))
}

fn plain_rect(r: &Box4) -> String {
    format!("{},{} {}×{}", num(r[0]), num(r[1]), num(r[2]), num(r[3]))
}

/// Токен, если значение пришло из темы, иначе само значение.
fn color_ref(look: &Look, key: &str, fallback: String) -> String {
    look.token(key).map_or(fallback, String::from)
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

// ─── Структура дампа ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dump {
    pub stage: StageInfo,
    pub fonts: FontsInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<TimeInfo>,
    /// Сцена снята: в кадре ничего нет.
    #[serde(skip_serializing_if = "is_false")]
    pub removed: bool,
    pub nodes: Vec<DumpNode>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageInfo {
    pub size: [f32; 2],
    pub fps: f32,
    pub action_safe: Box4,
    pub title_safe: Box4,
}

#[derive(Debug, Clone, Serialize)]
pub struct FontsInfo {
    pub files: Vec<FontFileInfo>,
    /// Разрешены ли системные шрифты.
    pub system: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FontFileInfo {
    pub families: Vec<String>,
    pub file: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimeInfo {
    pub t: f64,
    pub frame: u64,
    /// Фаза сцены: `loaded`, `in`, `live`, `out`, `removed`.
    pub scene: String,
    /// Обработанные команды сценария.
    pub log: Vec<String>,
    /// События до момента кадра включительно.
    pub events: Vec<String>,
}

/// Почему узла нет в кадре.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Absent {
    /// `when` ложно.
    Hidden,
    /// Сцена снята.
    SceneRemoved,
    /// Ушёл вместе с родителем.
    WithParent,
    /// Закончил `out`.
    Gone,
}

impl Absent {
    fn label(self) -> &'static str {
        match self {
            Absent::Hidden => "hidden: when=false",
            Absent::SceneRemoved => "сцена снята",
            Absent::WithParent => "вместе с родителем",
            Absent::Gone => "ушёл после out",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpNode {
    /// Адрес узла: `LowerThird#1/title`.
    pub path: String,
    pub kind: &'static str,
    /// Экземпляр компонента, если узел — его корень (`Scoreboard#1`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<LayoutInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rect: Option<Box4>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub absent: Option<Absent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<TextInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border: Option<BorderInfo>,
    #[serde(skip_serializing_if = "is_zero")]
    pub shadows: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    #[serde(skip_serializing_if = "is_false")]
    pub transform: bool,
    /// Слой presence/timeline: сдвиги и множители поверх look.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub motion: Option<RelMotion>,
    /// Фазы движения: `in 22%`, `loop`, `transition background 25%`, `timeline loop τ=1.8`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub phases: Vec<String>,
    /// Объявленный presence (дамп без часов).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub must: Option<Vec<String>>,
    /// Число шагов `else`.
    #[serde(rename = "else", skip_serializing_if = "is_zero")]
    pub else_steps: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied_else: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<DumpNode>,
    /// Сколько потомков отрезал фильтр `--depth`.
    #[serde(skip_serializing_if = "is_zero")]
    pub cut: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct LayoutInfo {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap: Option<f32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextInfo {
    pub content: String,
    /// Шрифт, которым набран текст.
    pub family: String,
    /// Запрошенное семейство, если набрано другим.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested: Option<String>,
    pub weight: f32,
    pub size: f32,
    /// Кегль до подбора `fit`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit_from: Option<f32>,
    #[serde(skip_serializing_if = "is_false")]
    pub truncated: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub clip: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub overflow: bool,
    pub color: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub spans: usize,
    pub lines: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct BorderInfo {
    pub width: f32,
    pub paint: String,
    /// Слой `on(...)`, если рамка включена условием.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on: Option<String>,
}

// ─── Построение дампа ───────────────────────────────────────────────────────

fn time_info(frame: &Frame) -> Option<TimeInfo> {
    frame.clock.as_ref().map(|c| TimeInfo { t: c.t, frame: c.frame, scene: c.scene.to_string(), log: c.log.clone(), events: c.events.clone() })
}

pub fn build(scene: &Scene, frame: &Frame) -> Dump {
    let nodes = match &scene.stage {
        Some(stage) => scene.node(stage.id).children.iter().map(|&c| node(scene, frame, c)).collect(),
        None => Vec::new(),
    };
    let mut diagnostics = frame.diagnostics.0.clone();
    diagnostics.sort_by_key(|d| std::cmp::Reverse(d.level));
    Dump {
        stage: StageInfo { size: frame.size, fps: frame.fps, action_safe: b4(&frame.action_safe), title_safe: b4(&frame.title_safe) },
        fonts: FontsInfo {
            files: frame.fonts.iter().map(|f| FontFileInfo { families: f.families.clone(), file: f.file.clone() }).collect(),
            system: frame.system_fonts,
        },
        time: time_info(frame),
        removed: frame.removed,
        nodes,
        diagnostics,
    }
}

fn absent(scene: &Scene, frame: &Frame, id: u32) -> Absent {
    let node = scene.node(id);
    if node.hidden() {
        Absent::Hidden
    } else if frame.removed {
        Absent::SceneRemoved
    } else if node.parent.is_some_and(|p| scene.node(p).kind != Kind::Stage && frame.find(p).is_none()) {
        Absent::WithParent
    } else {
        Absent::Gone
    }
}

fn keys(v: &Value) -> Vec<String> {
    v.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default()
}

fn node(scene: &Scene, frame: &Frame, id: u32) -> DumpNode {
    let n = scene.node(id);
    let fnode = frame.find(id);
    let layout = n.props.get("layout").and_then(|l| {
        let kind = l.get("kind")?.as_str()?.to_string();
        Some(LayoutInfo { kind, gap: l.get("gap").and_then(Value::as_f64).map(|g| g as f32) })
    });
    let mut out = DumpNode {
        path: scene.path(id),
        kind: n.kind.name(),
        instance: n.instance().map(String::from),
        layout,
        rect: fnode.map(|f| b4(&f.rect)),
        absent: if fnode.is_none() { Some(absent(scene, frame, id)) } else { None },
        text: None,
        background: None,
        radius: None,
        padding: None,
        border: None,
        shadows: 0,
        opacity: None,
        transform: false,
        motion: None,
        phases: Vec::new(),
        presence: None,
        must: n.props.get("must").map(keys),
        else_steps: n.props.get("else").and_then(Value::as_array).map_or(0, Vec::len),
        applied_else: fnode.and_then(|f| f.applied_else.clone()),
        children: n.children.iter().map(|&c| node(scene, frame, c)).collect(),
        cut: 0,
    };
    if let Some(f) = fnode {
        visual(frame, f, &mut out);
    }
    if let Some(a) = fnode.and_then(|f| f.anim.as_ref()) {
        if !a.motion.is_identity() {
            out.motion = Some(a.motion);
        }
        out.phases = a.notes.clone();
    } else if frame.clock.is_none() && let Some(p) = n.props.get("presence") {
        out.presence = Some(keys(p));
    }
    out
}

fn visual(frame: &Frame, f: &FrameNode, out: &mut DumpNode) {
    let v = &f.visual;
    if let Some((index, _)) = f.text {
        let block = &frame.texts[index];
        let look = block.look();
        let family = block.resolved_family.clone().unwrap_or_else(|| "?".into());
        out.text = Some(TextInfo {
            content: block.content.clone(),
            requested: look.family.clone().filter(|r| !r.eq_ignore_ascii_case(&family)),
            family,
            weight: look.weight,
            // Итоговый кегль после подбора; исходный — в fit_from.
            size: block.size,
            fit_from: (block.size < look.size).then_some(look.size),
            truncated: block.truncated,
            clip: block.clip,
            overflow: block.overflow,
            color: color_ref(&f.look, "color", look.color.to_string()),
            spans: block.source.runs.iter().filter(|r| r.style.is_some()).count(),
            lines: block.line_count(),
        });
    }
    out.background = v.background.map(|bg| color_ref(&f.look, "background", bg.to_string()));
    out.radius = (v.radius != [0.0; 4]).then_some(v.radius);
    out.padding = (v.padding != [0.0; 4]).then_some(v.padding);
    out.border = v.border.as_ref().map(|b| {
        let rule = f.look.props.get("border");
        BorderInfo {
            width: b.width,
            paint: rule.and_then(|r| r.prov.tokens.first().cloned()).unwrap_or_else(|| b.color.to_string()),
            on: rule.map(|r| r.prov.layer.clone()).filter(|l| l.ends_with(" on")),
        }
    });
    out.shadows = v.shadows.len();
    out.opacity = (v.opacity < 1.0).then_some(v.opacity);
    out.transform = v.transform.is_some();
}

// ─── Фильтры ────────────────────────────────────────────────────────────────

fn count(nodes: &[DumpNode]) -> usize {
    nodes.iter().map(|n| 1 + count(&n.children)).sum()
}

fn find_node<'a>(nodes: &'a [DumpNode], path: &str) -> Option<&'a DumpNode> {
    nodes.iter().find_map(|n| if n.path == path || n.instance.as_deref() == Some(path) { Some(n) } else { find_node(&n.children, path) })
}

fn cut(node: &mut DumpNode, depth: usize) {
    if depth == 0 {
        node.cut = count(&node.children);
        node.children.clear();
    } else {
        for c in &mut node.children {
            cut(c, depth - 1);
        }
    }
}

impl Dump {
    /// Поддерево узла `path` и глубина: 0 — только сам узел, 1 — с детьми и т. д.
    /// Диагностики остаются только для узлов поддерева.
    pub fn filter(&mut self, path: Option<&str>, depth: Option<usize>) -> Result<(), String> {
        if let Some(path) = path {
            let Some(found) = find_node(&self.nodes, path).cloned() else {
                return Err(format!("узел {path} не найден; адреса узлов есть в `kine dump`"));
            };
            let mut inside = Vec::new();
            fn walk(n: &DumpNode, out: &mut Vec<String>) {
                out.push(n.path.clone());
                n.children.iter().for_each(|c| walk(c, out));
            }
            walk(&found, &mut inside);
            self.nodes = vec![found];
            self.diagnostics.retain(|d| inside.contains(&d.node));
        }
        if let Some(depth) = depth {
            for n in &mut self.nodes {
                cut(n, depth);
            }
        }
        Ok(())
    }

    /// Все адреса узлов в порядке обхода.
    pub fn paths(&self) -> Vec<String> {
        fn walk(nodes: &[DumpNode], out: &mut Vec<String>) {
            for n in nodes {
                out.push(n.path.clone());
                walk(&n.children, out);
            }
        }
        let mut out = Vec::new();
        walk(&self.nodes, &mut out);
        out
    }
}

// ─── Текст дампа ────────────────────────────────────────────────────────────

pub fn dump(scene: &Scene, frame: &Frame) -> String {
    text(&build(scene, frame))
}

pub fn text(d: &Dump) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "stage {}×{} @{}fps · actionSafe {} · titleSafe {}",
        num(d.stage.size[0]),
        num(d.stage.size[1]),
        num(d.stage.fps),
        plain_rect(&d.stage.action_safe),
        plain_rect(&d.stage.title_safe)
    );
    let fonts: Vec<String> = d.fonts.files.iter().map(|f| format!("{} ({})", f.families.join(", "), f.file)).collect();
    let _ = writeln!(
        out,
        "fonts: {}{}",
        if fonts.is_empty() { "—".to_string() } else { fonts.join(" · ") },
        if d.fonts.system { " + системные" } else { " · системные выключены" }
    );
    if let Some(c) = &d.time {
        let log = if c.log.is_empty() { String::new() } else { format!(" · {}", c.log.join(", ")) };
        let _ = writeln!(out, "time: t={}s · кадр {} · сцена {}{log}", num(c.t as f32), c.frame, c.scene);
        // Последние события: что уже произошло к этому кадру (маркеры, состояния, emit).
        const SHOWN: usize = 8;
        if !c.events.is_empty() {
            let skip = c.events.len().saturating_sub(SHOWN);
            let earlier = if skip > 0 { format!(" (+{skip} ранее)") } else { String::new() };
            let _ = writeln!(out, "events{earlier}: {}", c.events[skip..].join(" · "));
        }
    }
    if d.removed {
        let _ = writeln!(out, "сцена снята: в кадре ничего нет");
    }
    for (i, n) in d.nodes.iter().enumerate() {
        node_lines(n, "", i + 1 == d.nodes.len(), &mut out);
    }
    if !d.diagnostics.is_empty() {
        let count = |l: Level| d.diagnostics.iter().filter(|x| x.level == l).count();
        let _ = writeln!(out, "\ndiagnostics: {} error, {} warn, {} info", count(Level::Error), count(Level::Warn), count(Level::Info));
        for x in &d.diagnostics {
            let _ = writeln!(out, "{x}");
        }
    }
    out
}

fn node_lines(n: &DumpNode, prefix: &str, last: bool, out: &mut String) {
    let branch = if last { "└─ " } else { "├─ " };
    let mut line = format!("{prefix}{branch}{}  {}", n.path, n.kind);
    if let Some(l) = &n.layout {
        let _ = write!(line, " {}", l.kind);
        if let Some(gap) = l.gap {
            let _ = write!(line, " gap {}", num(gap));
        }
    }
    match (&n.rect, n.absent) {
        (Some(r), _) => {
            let _ = write!(line, "  {}", rect(r));
            summary(n, &mut line);
        }
        (None, Some(a)) => {
            let _ = write!(line, "  ({})", a.label());
        }
        (None, None) => {}
    }
    if let Some(m) = &n.motion {
        let _ = write!(line, "  motion {}", m.describe().join(" · "));
    }
    if !n.phases.is_empty() {
        let _ = write!(line, "  [{}]", n.phases.join(", "));
    }
    if let Some(p) = &n.presence {
        let _ = write!(line, "  [presence {}]", p.join(","));
    }
    if let Some(m) = &n.must {
        let _ = write!(line, "  [must {}]", m.join(","));
    }
    if n.else_steps > 0 {
        let _ = write!(line, "  [else×{}]", n.else_steps);
    }
    if let Some(label) = &n.applied_else {
        let _ = write!(line, "  [применён {label}]");
    }
    if n.cut > 0 {
        let _ = write!(line, "  [+{} вложенных, см. --depth]", n.cut);
    }
    let _ = writeln!(out, "{line}");

    let child_prefix = format!("{prefix}{}", if last { "   " } else { "│  " });
    for (i, c) in n.children.iter().enumerate() {
        node_lines(c, &child_prefix, i + 1 == n.children.len(), out);
    }
}

fn summary(n: &DumpNode, line: &mut String) {
    let mut parts: Vec<String> = Vec::new();
    if let Some(t) = &n.text {
        let mut text = format!("{:?}  {} {} {}px", t.content, t.family, num(t.weight), num(t.fit_from.unwrap_or(t.size)));
        if let Some(requested) = &t.requested {
            let _ = write!(text, " (запрошен {requested})");
        }
        if let Some(from) = t.fit_from {
            let _ = write!(text, " (fit {}→{})", num(from), num(t.size));
        }
        if t.truncated {
            text.push_str(" (обрезан)");
        }
        if t.clip {
            text.push_str(" (clip)");
        }
        if t.overflow {
            text.push_str(" (не помещается)");
        }
        parts.push(text);
        parts.push(t.color.clone());
        if t.spans > 0 {
            parts.push(format!("span×{}", t.spans));
        }
        if t.lines > 1 {
            parts.push(format!("строк: {}", t.lines));
        }
    }
    if let Some(bg) = &n.background {
        parts.push(format!("bg {bg}"));
    }
    if let Some(r) = n.radius {
        let r = if r.iter().all(|x| *x == r[0]) { num(r[0]) } else { r.map(num).join(",") };
        parts.push(format!("r {r}"));
    }
    if let Some([t, r, b, l]) = n.padding {
        let p = if t == b && r == l { format!("{},{}", num(t), num(r)) } else { [t, r, b, l].map(num).join(",") };
        parts.push(format!("pad {p}"));
    }
    if let Some(b) = &n.border {
        let mut s = format!("border {} {}", num(b.width), b.paint);
        if let Some(on) = &b.on {
            let _ = write!(s, " ({on})");
        }
        parts.push(s);
    }
    if n.shadows > 0 {
        parts.push(format!("shadow×{}", n.shadows));
    }
    if let Some(o) = n.opacity {
        parts.push(format!("opacity {}", num(o)));
    }
    if n.transform {
        parts.push("transform".into());
    }
    if !parts.is_empty() {
        let _ = write!(line, "  {}", parts.join(" · "));
    }
}

// ─── Inspect ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inspect {
    pub path: String,
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    /// Место в исходнике TSX.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<TimeInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub absent: Option<Absent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rect: Option<Box4>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied_else: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<InspectText>,
    /// Итоговые значения `look` с происхождением.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub look: Vec<LookEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub motion: Option<InspectMotion>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub props: BTreeMap<String, Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectText {
    pub content: String,
    pub family: String,
    pub requested: String,
    pub size: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit_from: Option<f32>,
    pub lines: usize,
    /// Размер набранного текста.
    pub measured: [f32; 2],
    /// Контентный блок узла.
    pub content_box: Box4,
    #[serde(skip_serializing_if = "is_false")]
    pub truncated: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub overflow: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub clip: bool,
    /// Куски текста; заполняется, если есть Span.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub runs: Vec<RunInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunInfo {
    pub text: String,
    /// Стиль Span; `None` — обычный текст.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LookEntry {
    pub key: String,
    pub value: String,
    /// Происхождение: токены темы и слой look, от ближнего к дальнему.
    pub from: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InspectMotion {
    pub phases: Vec<String>,
    pub channels: Vec<ChannelEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChannelEntry {
    pub channel: String,
    pub value: String,
    /// `presence`, `transition`, `timeline`.
    pub layer: String,
    pub source: String,
    pub easing: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
}

pub fn inspect_build(scene: &Scene, frame: &Frame, path: &str) -> Option<Inspect> {
    let id = scene.find(path)?;
    let node = scene.node(id);
    let f = frame.find(id);
    let mut out = Inspect {
        path: scene.path(id),
        kind: node.kind.name(),
        instance: node.instance().map(String::from),
        source: node.src.clone(),
        time: time_info(frame),
        absent: if f.is_none() { Some(absent(scene, frame, id)) } else { None },
        rect: f.map(|f| b4(&f.rect)),
        applied_else: f.and_then(|f| f.applied_else.clone()),
        text: None,
        look: Vec::new(),
        motion: None,
        props: BTreeMap::new(),
        children: Vec::new(),
        diagnostics: Vec::new(),
    };
    let Some(f) = f else { return Some(out) };

    if let Some((index, content)) = f.text {
        let block = &frame.texts[index];
        let look = block.look();
        let mut runs = Vec::new();
        if node.runs.iter().any(|r| r.look.is_some()) {
            for (i, run) in node.runs.iter().enumerate() {
                let span = block.source.runs.get(i).and_then(|r| r.style.as_ref()).map(|st| {
                    let mut parts = Vec::new();
                    if let Some(c) = st.color {
                        parts.push(format!("color {c}"));
                    }
                    if let Some(v) = st.size {
                        parts.push(format!("size {}", num(v)));
                    }
                    if let Some(v) = st.weight {
                        parts.push(format!("weight {}", num(v)));
                    }
                    if let Some(v) = &st.family {
                        parts.push(format!("font {v}"));
                    }
                    if let Some(v) = st.tracking {
                        parts.push(format!("tracking {}", num(v)));
                    }
                    if st.case.is_some() {
                        parts.push("case".into());
                    }
                    parts
                });
                runs.push(RunInfo { text: run.text.clone(), span, source: run.src.clone() });
            }
        }
        out.text = Some(InspectText {
            content: block.content.clone(),
            family: block.resolved_family.clone().unwrap_or_else(|| "?".into()),
            requested: look.family.clone().unwrap_or_else(|| "sans-serif".into()),
            size: block.size,
            fit_from: (block.size < look.size).then_some(look.size),
            lines: block.line_count(),
            measured: [block.layout.width(), block.layout.height()],
            content_box: b4(&content),
            truncated: block.truncated,
            overflow: block.overflow,
            clip: block.clip,
            runs,
        });
    }

    for (key, r) in &f.look.props {
        let mut from: Vec<String> = r.prov.tokens.iter().map(|t| format!("token {t}")).collect();
        from.push(r.prov.layer.clone());
        out.look.push(LookEntry { key: key.clone(), value: r.value.to_string(), from });
    }

    if let Some(a) = &f.anim
        && (!a.channels.is_empty() || !a.notes.is_empty())
    {
        out.motion = Some(InspectMotion {
            phases: a.notes.clone(),
            channels: a
                .channels
                .iter()
                .map(|c| ChannelEntry {
                    channel: c.channel.into(),
                    value: c.value.clone(),
                    layer: c.layer.into(),
                    source: c.source.into(),
                    easing: c.easing.clone(),
                    progress: c.progress,
                })
                .collect(),
        });
    }

    let skip = ["look", "instance", "part", "hidden"];
    out.props = node.props.iter().filter(|(k, _)| !skip.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
    if node.kind != Kind::Text {
        out.children = node.children.iter().map(|c| scene.path(*c)).collect();
    }
    out.diagnostics = frame.diagnostics.0.iter().filter(|d| d.node == out.path).cloned().collect();
    Some(out)
}

pub fn inspect(scene: &Scene, frame: &Frame, path: &str) -> Option<String> {
    inspect_build(scene, frame, path).map(|i| inspect_text(&i))
}

pub fn inspect_text(i: &Inspect) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{}  {}", i.path, i.kind);
    if let Some(inst) = &i.instance {
        let _ = writeln!(out, "instance  {inst}");
    }
    if let Some(src) = &i.source {
        let _ = writeln!(out, "source    {src}");
    }
    if let Some(c) = &i.time {
        let _ = writeln!(out, "time      t={}s, кадр {}, сцена {}", num(c.t as f32), c.frame, c.scene);
    }
    if let Some(a) = i.absent {
        let why = match a {
            Absent::Hidden => "when=false",
            other => other.label(),
        };
        let _ = writeln!(out, "hidden    {why}, в кадре отсутствует");
        return out;
    }
    if let Some(r) = &i.rect {
        let _ = writeln!(out, "rect      {}", plain_rect(r));
    }
    if let Some(label) = &i.applied_else {
        let _ = writeln!(out, "else      применён {label}");
    }
    if let Some(t) = &i.text {
        let _ = writeln!(out, "content   {:?}", t.content);
        let _ = writeln!(
            out,
            "font      {} (запрошен {}), {}px{}, строк: {}, текст {}×{} в блоке {}",
            t.family,
            t.requested,
            num(t.size),
            t.fit_from.map(|f| format!(" (fit из {})", num(f))).unwrap_or_default(),
            t.lines,
            num(t.measured[0]),
            num(t.measured[1]),
            plain_rect(&t.content_box)
        );
        let state: Vec<&str> = [(t.truncated, "обрезан с многоточием"), (t.overflow, "не помещается"), (t.clip, "обрезан по блоку")]
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, s)| *s)
            .collect();
        if !state.is_empty() {
            let _ = writeln!(out, "fit       {}", state.join(", "));
        }
        if !t.runs.is_empty() {
            let _ = writeln!(out, "runs");
            for run in &t.runs {
                match &run.span {
                    Some(parts) => {
                        let src = run.source.as_deref().map(|s| format!(" @ {s}")).unwrap_or_default();
                        let _ = writeln!(out, "  span {:?}  {}{src}", run.text, parts.join(" · "));
                    }
                    None => {
                        let _ = writeln!(out, "  text {:?}", run.text);
                    }
                }
            }
        }
    }
    if !i.look.is_empty() {
        let _ = writeln!(out, "look");
        for e in &i.look {
            let _ = writeln!(out, "  {:<11} {:<40} ← {}", e.key, e.value, e.from.join(" ← "));
        }
    }
    if let Some(m) = &i.motion {
        let _ = writeln!(out, "motion    {}", m.phases.join(", "));
        for c in &m.channels {
            let progress = c.progress.map(|p| format!(" {}%", (p * 100.0).round() as i64)).unwrap_or_default();
            let origin = if c.layer == c.source { c.layer.clone() } else { format!("{} {}", c.layer, c.source) };
            let _ = writeln!(out, "  {:<11} {:<10} ← {origin} ({}){progress}", c.channel, c.value, c.easing);
        }
    }
    if !i.props.is_empty() {
        let _ = writeln!(out, "props");
        for (k, v) in &i.props {
            let _ = writeln!(out, "  {k:<11} {v}");
        }
    }
    if !i.children.is_empty() {
        let _ = writeln!(out, "children  {}", i.children.join(", "));
    }
    if !i.diagnostics.is_empty() {
        let _ = writeln!(out, "diagnostics");
        for d in &i.diagnostics {
            let _ = writeln!(out, "  {d}");
        }
    }
    out
}
