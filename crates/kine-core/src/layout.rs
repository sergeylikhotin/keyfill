//! Сборка кадра: разрешённые стили, раскладка Taffy, якоря верхнего уровня, текст.

use anyhow::{Result, bail};
use serde_json::Value;
use taffy::prelude::*;

use crate::diag::{Diagnostics, Level};
use crate::look::{self, Look, Visual};
use crate::scene::{Kind, Scene};
use crate::text::{TextBlock, TextEngine};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn inset(self, frac: f32) -> Rect {
        let dx = self.w * (1.0 - frac) / 2.0;
        let dy = self.h * (1.0 - frac) / 2.0;
        Rect { x: self.x + dx, y: self.y + dy, w: self.w - 2.0 * dx, h: self.h - 2.0 * dy }
    }

    pub fn contains(&self, other: &Rect) -> bool {
        const EPS: f32 = 0.01;
        other.x >= self.x - EPS
            && other.y >= self.y - EPS
            && other.x + other.w <= self.x + self.w + EPS
            && other.y + other.h <= self.y + self.h + EPS
    }
}

pub struct FrameNode {
    pub id: u32,
    pub rect: Rect,
    pub look: Look,
    pub visual: Visual,
    /// Индекс в `Frame::texts` и начало контентного блока.
    pub text: Option<(usize, [f32; 2])>,
    pub children: Vec<FrameNode>,
}

pub struct Frame {
    pub size: [f32; 2],
    pub fps: f32,
    pub action_safe: Rect,
    pub title_safe: Rect,
    pub roots: Vec<FrameNode>,
    pub texts: Vec<TextBlock>,
    pub diagnostics: Diagnostics,
}

impl Frame {
    pub fn find(&self, id: u32) -> Option<&FrameNode> {
        fn walk(n: &FrameNode, id: u32) -> Option<&FrameNode> {
            if n.id == id {
                return Some(n);
            }
            n.children.iter().find_map(|c| walk(c, id))
        }
        self.roots.iter().find_map(|r| walk(r, id))
    }
}

struct Pending {
    id: u32,
    tnode: NodeId,
    look: Look,
    visual: Visual,
    text: Option<usize>,
    children: Vec<Pending>,
}

pub fn build_frame(scene: &Scene, engine: &mut TextEngine) -> Result<Frame> {
    let Some(stage) = scene.stage.clone() else {
        bail!("в сцене нет stage: бандл должен вызвать mount(scene)");
    };
    let stage_rect = Rect { x: 0.0, y: 0.0, w: stage.size[0], h: stage.size[1] };
    let mut frame = Frame {
        size: stage.size,
        fps: stage.fps,
        action_safe: stage_rect.inset(stage.action),
        title_safe: stage_rect.inset(stage.title),
        roots: Vec::new(),
        texts: Vec::new(),
        diagnostics: Diagnostics::default(),
    };

    let mut taffy: TaffyTree<usize> = TaffyTree::new();
    let stage_children: Vec<u32> = scene.node(stage.id).children.clone();

    for child in stage_children {
        if scene.node(child).hidden() {
            continue;
        }
        let mut pending = build_node(scene, child, &mut taffy, engine, &mut frame)?;
        let anchor = scene.node(child).props.get("anchor").cloned();

        let target = match &anchor {
            Some(a) => anchor_target(a, (frame.action_safe, frame.title_safe), stage_rect, scene, child, &mut frame.diagnostics),
            None => stage_rect,
        };
        let mut style = taffy.style(pending.tnode)?.clone();
        if anchor.is_some() {
            style.max_size = Size { width: length(target.w), height: length(target.h) };
            taffy.set_style(pending.tnode, style)?;
        } else if style.size.width.is_auto() && style.size.height.is_auto() {
            style.size = Size { width: length(stage_rect.w), height: length(stage_rect.h) };
            taffy.set_style(pending.tnode, style)?;
        }

        let texts = &mut frame.texts;
        taffy.compute_layout_with_measure(pending.tnode, Size::MAX_CONTENT, |inputs, _id, ctx, style| {
            taffy::compute_leaf_layout(inputs, style, |_, _| 0.0, |known, available| match ctx {
                Some(index) => measure_text(&mut texts[*index], known, available),
                None => Size::ZERO,
            })
        })?;

        let size = taffy.layout(pending.tnode)?.size;
        let origin = match &anchor {
            Some(a) => place(a, target, size.width, size.height),
            None => [0.0, 0.0],
        };
        let root = finish(&taffy, &mut pending, origin, &mut frame)?;
        if let Some(a) = &anchor {
            check_anchor_inside(scene, &root, a, target, &mut frame.diagnostics);
        }
        frame.roots.push(root);
    }
    Ok(frame)
}

fn build_node(scene: &Scene, id: u32, taffy: &mut TaffyTree<usize>, engine: &mut TextEngine, frame: &mut Frame) -> Result<Pending> {
    let node = scene.node(id);
    let path = scene.path(id);
    let src = node.src.as_deref();
    let diags = &mut frame.diagnostics;

    let look = look::resolve(scene, node, diags);
    let visual = look::visual(&look, &path, src, diags);
    let style = taffy_style(scene, id, &visual, diags);

    if node.props.contains_key("anchor") && node.parent.and_then(|p| scene.nodes.get(&p)).is_some_and(|p| p.kind != Kind::Stage) {
        diags.push(Level::Info, "m0.unsupported", &path, src, "anchor внутри контейнера применяется с M1", None);
    }

    match node.kind {
        Kind::Text => {
            let tl = look::text_look(&look, &path, src, diags);
            let block = engine.build(&node.text, &tl);
            if let (Some(requested), Some(resolved)) = (&tl.family, &block.resolved_family)
                && !requested.eq_ignore_ascii_case(resolved) {
                    diags.push(
                        Level::Warn,
                        "font.fallback",
                        &path,
                        src,
                        format!("шрифт {requested} не найден, использован {resolved}"),
                        Some("установи шрифт в систему или подключи файл через font: { family, file } (файлы шрифтов — M1)"),
                    );
                }
            frame.texts.push(block);
            let index = frame.texts.len() - 1;
            let tnode = taffy.new_leaf_with_context(style, index)?;
            Ok(Pending { id, tnode, look, visual, text: Some(index), children: Vec::new() })
        }
        Kind::Box | Kind::Image | Kind::Shape | Kind::Stage => {
            if matches!(node.kind, Kind::Image | Kind::Shape) {
                diags.push(Level::Info, "m0.unsupported", &path, src, format!("{} рисуется с M1", node.kind.name()), None);
            }
            let mut children = Vec::new();
            for &c in &node.children {
                if !scene.node(c).hidden() {
                    children.push(build_node(scene, c, taffy, engine, frame)?);
                }
            }
            let ids: Vec<NodeId> = children.iter().map(|c| c.tnode).collect();
            let tnode = taffy.new_with_children(style, &ids)?;
            Ok(Pending { id, tnode, look, visual, text: None, children })
        }
    }
}

fn measure_text(block: &mut TextBlock, known: Size<Option<f32>>, available: Size<AvailableSpace>) -> Size<f32> {
    let max = known.width.or(match available.width {
        AvailableSpace::Definite(w) => Some(w),
        AvailableSpace::MinContent => Some(0.0),
        AvailableSpace::MaxContent => None,
    });
    block.layout.break_all_lines(max);
    Size {
        width: known.width.unwrap_or(block.layout.width().ceil()),
        height: known.height.unwrap_or(block.layout.height().ceil()),
    }
}

fn finish(taffy: &TaffyTree<usize>, p: &mut Pending, origin: [f32; 2], frame: &mut Frame) -> Result<FrameNode> {
    let layout = taffy.layout(p.tnode)?;
    let rect = Rect { x: origin[0] + layout.location.x, y: origin[1] + layout.location.y, w: layout.size.width, h: layout.size.height };

    let text = match p.text {
        Some(index) => {
            let border = p.visual.border.as_ref().map_or(0.0, |b| b.width);
            let [pt, pr, _, pl] = p.visual.padding;
            let content_w = (rect.w - pl - pr - 2.0 * border).max(0.0);
            frame.texts[index].finalize(content_w);
            Some((index, [rect.x + border + pl, rect.y + border + pt]))
        }
        None => None,
    };

    let mut children = Vec::with_capacity(p.children.len());
    for c in &mut p.children {
        children.push(finish(taffy, c, [rect.x, rect.y], frame)?);
    }
    Ok(FrameNode { id: p.id, rect, look: std::mem::take(&mut p.look), visual: p.visual.clone(), text, children })
}

// ─── Стиль Taffy ────────────────────────────────────────────────────────────

fn taffy_style(scene: &Scene, id: u32, visual: &Visual, diags: &mut Diagnostics) -> Style {
    let node = scene.node(id);
    let path = scene.path(id);
    let src = node.src.as_deref();
    let mut s = Style { display: Display::Flex, flex_direction: FlexDirection::Column, ..Default::default() };

    if let Some(layout) = node.props.get("layout") {
        match layout.get("kind").and_then(Value::as_str) {
            Some("row") => s.flex_direction = FlexDirection::Row,
            Some("column") => {}
            Some(other) => diags.push(Level::Info, "m0.unsupported", &path, src, format!("layout {other} появится в M1, разложено как column"), None),
            None => diags.push(Level::Error, "layout.invalid", &path, src, "layout без kind", Some("используй row(), column(), grid() или stack()")),
        }
        if let Some(gap) = layout.get("gap").and_then(Value::as_f64) {
            s.gap = Size { width: length(gap as f32), height: length(gap as f32) };
        }
        if let Some(a) = layout.get("align").and_then(Value::as_str).and_then(|a| match a {
            "start" => Some(AlignItems::START),
            "center" => Some(AlignItems::CENTER),
            "end" => Some(AlignItems::END),
            "stretch" => Some(AlignItems::STRETCH),
            "baseline" => Some(AlignItems::BASELINE),
            _ => None,
        }) {
            s.align_items = a;
        }
        if let Some(j) = layout.get("justify").and_then(Value::as_str).and_then(|j| match j {
            "start" => Some(JustifyContent::START),
            "center" => Some(JustifyContent::CENTER),
            "end" => Some(JustifyContent::END),
            "between" => Some(JustifyContent::SPACE_BETWEEN),
            "around" => Some(JustifyContent::SPACE_AROUND),
            "evenly" => Some(JustifyContent::SPACE_EVENLY),
            _ => None,
        }) {
            s.justify_content = j;
        }
        if layout.get("wrap").and_then(Value::as_bool) == Some(true) {
            s.flex_wrap = FlexWrap::Wrap;
        }
    }

    let [t, r, b, l] = visual.padding;
    s.padding = taffy::Rect { top: length(t), right: length(r), bottom: length(b), left: length(l) };
    if let Some(border) = &visual.border {
        let w = border.width;
        s.border = taffy::Rect { top: length(w), right: length(w), bottom: length(w), left: length(w) };
    }

    let mut dims = |key: &str| -> Size<Option<SizeValue>> {
        let Some(v) = node.props.get(key) else { return Size { width: None, height: None } };
        let mut dim = |k: &str| {
            let d = v.get(k)?;
            match parse_size_value(d) {
                Ok(d) => Some(d),
                Err(e) => {
                    diags.push(Level::Error, "layout.invalid", &path, src, format!("{key}.{k}: {e}"), None);
                    None
                }
            }
        };
        Size { width: dim("w"), height: dim("h") }
    };
    let size = dims("size");
    s.size = size.map(|d| d.map_or(Dimension::auto(), SizeValue::dimension));
    s.min_size = dims("min").map(|d| d.map_or(LengthPercentageAuto::auto(), SizeValue::limit));
    s.max_size = dims("max").map(|d| d.map_or(LengthPercentageAuto::auto(), SizeValue::limit));
    if let Some(g) = node.props.get("grow").and_then(Value::as_f64) {
        s.flex_grow = g as f32;
    }
    if let Some(g) = node.props.get("shrink").and_then(Value::as_f64) {
        s.flex_shrink = g as f32;
    }
    s.aspect_ratio = node.props.get("aspect").and_then(Value::as_f64).map(|a| a as f32);
    s
}

#[derive(Debug, Clone, Copy)]
enum SizeValue {
    Len(f32),
    Pct(f32),
    Auto,
    Content,
}

impl SizeValue {
    fn dimension(self) -> Dimension {
        match self {
            SizeValue::Len(v) => Dimension::length(v),
            SizeValue::Pct(p) => Dimension::percent(p / 100.0),
            SizeValue::Auto => Dimension::auto(),
            SizeValue::Content => Dimension::fit_content(),
        }
    }

    /// Для min/max Taffy принимает только длину, процент или auto.
    fn limit(self) -> LengthPercentageAuto {
        match self {
            SizeValue::Len(v) => LengthPercentageAuto::length(v),
            SizeValue::Pct(p) => LengthPercentageAuto::percent(p / 100.0),
            SizeValue::Auto | SizeValue::Content => LengthPercentageAuto::auto(),
        }
    }
}

fn parse_size_value(v: &Value) -> Result<SizeValue, String> {
    if let Some(n) = v.as_f64() {
        return Ok(SizeValue::Len(n as f32));
    }
    if let Some(p) = v.get("pct").and_then(Value::as_f64) {
        return Ok(SizeValue::Pct(p as f32));
    }
    match v.as_str() {
        Some("auto") => Ok(SizeValue::Auto),
        Some("content") => Ok(SizeValue::Content),
        _ => Err(format!("ожидается число, pct(n), 'auto' или 'content', получено {v}")),
    }
}

// ─── Якоря ──────────────────────────────────────────────────────────────────

fn point(name: &str) -> Option<[f32; 2]> {
    Some(match name {
        "top-left" => [0.0, 0.0],
        "top" => [0.5, 0.0],
        "top-right" => [1.0, 0.0],
        "left" => [0.0, 0.5],
        "center" => [0.5, 0.5],
        "right" => [1.0, 0.5],
        "bottom-left" => [0.0, 1.0],
        "bottom" => [0.5, 1.0],
        "bottom-right" => [1.0, 1.0],
        _ => return None,
    })
}

fn anchor_target(anchor: &Value, safe: (Rect, Rect), stage: Rect, scene: &Scene, id: u32, diags: &mut Diagnostics) -> Rect {
    match anchor.get("to").and_then(Value::as_str) {
        Some("stage") | Some("parent") => stage,
        Some("actionSafe") => safe.0,
        Some("titleSafe") => safe.1,
        other => {
            diags.push(
                Level::Info,
                "m0.unsupported",
                &scene.path(id),
                scene.node(id).src.as_deref(),
                format!("anchor.to {other:?}: привязка к частям появится в M1, использован stage"),
                None,
            );
            stage
        }
    }
}

fn place(anchor: &Value, target: Rect, w: f32, h: f32) -> [f32; 2] {
    let at = anchor.get("at").and_then(Value::as_str).and_then(point).unwrap_or([0.0, 0.0]);
    let selfp = anchor.get("self").and_then(Value::as_str).and_then(point).unwrap_or(at);
    let offset = anchor
        .get("offset")
        .and_then(Value::as_array)
        .map(|o| [o[0].as_f64().unwrap_or(0.0) as f32, o[1].as_f64().unwrap_or(0.0) as f32])
        .unwrap_or([0.0, 0.0]);
    [
        target.x + at[0] * target.w - selfp[0] * w + offset[0],
        target.y + at[1] * target.h - selfp[1] * h + offset[1],
    ]
}

/// Якорь к safe area подразумевает, что узел в ней помещается; если нет — это сразу видно в дампе.
fn check_anchor_inside(scene: &Scene, root: &FrameNode, anchor: &Value, target: Rect, diags: &mut Diagnostics) {
    if target.contains(&root.rect) {
        return;
    }
    let to = anchor.get("to").and_then(Value::as_str).unwrap_or("?");
    diags.push(
        Level::Warn,
        "anchor.overflow",
        &scene.path(root.id),
        scene.node(root.id).src.as_deref(),
        format!("узел {}×{} не помещается в {to} {}×{}", look::num(root.rect.w), look::num(root.rect.h), look::num(target.w), look::num(target.h)),
        Some("ограничь размер (size/max) или уменьши содержимое"),
    );
}
