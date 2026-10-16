//! Сборка кадра: разрешённые стили, раскладка Taffy (flex, grid, stack), якоря, текст с `fit`.

use std::collections::HashMap;

use anyhow::{Result, bail};
use serde_json::Value;
use taffy::prelude::*;
use taffy::style_helpers::{FromFr, FromLength, FromPercent, TaffyAuto};

use crate::diag::{Diagnostics, Level};
use crate::look::{self, Look, Target, Transform, Visual};
use crate::motion::{Animated, Channel, Clock, NodeValues, V};
use crate::scene::{Kind, Scene};
use crate::text::{Fit, FitThen, LoadedFont, RunSpec, TextBlock, TextEngine, TextSource};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// `x,y w×h`, числа как в дампе.
impl std::fmt::Display for Rect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{},{} {}×{}", look::num(self.x), look::num(self.y), look::num(self.w), look::num(self.h))
    }
}

impl Rect {
    pub fn inset(self, frac: f32) -> Rect {
        let dx = self.w * (1.0 - frac) / 2.0;
        let dy = self.h * (1.0 - frac) / 2.0;
        Rect { x: self.x + dx, y: self.y + dy, w: self.w - 2.0 * dx, h: self.h - 2.0 * dy }
    }

    pub fn contains(&self, other: &Rect) -> bool {
        const EPS: f32 = 0.5;
        other.x >= self.x - EPS
            && other.y >= self.y - EPS
            && other.x + other.w <= self.x + self.w + EPS
            && other.y + other.h <= self.y + self.h + EPS
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        other.x < self.x + self.w && other.x + other.w > self.x && other.y < self.y + self.h && other.y + other.h > self.y
    }

    fn shifted(self, dx: f32, dy: f32) -> Rect {
        Rect { x: self.x + dx, y: self.y + dy, ..self }
    }
}

/// Контентный блок: прямоугольник узла минус рамка и отступы.
pub fn content_rect(rect: Rect, visual: &Visual) -> Rect {
    let border = visual.border.as_ref().map_or(0.0, |b| b.width);
    let [t, r, b, l] = visual.padding;
    Rect {
        x: rect.x + border + l,
        y: rect.y + border + t,
        w: (rect.w - l - r - 2.0 * border).max(0.0),
        h: (rect.h - t - b - 2.0 * border).max(0.0),
    }
}

/// Вариант `else`. Варианты независимые: применяется ровно один, поверх собственных fit и look узла.
#[derive(Debug, Clone, Default)]
pub struct ElseStep {
    /// `else[k]`.
    pub label: String,
    pub shrink_to: Option<f32>,
    pub lines: Option<usize>,
    pub ellipsis: bool,
    pub look: Vec<Value>,
    pub hide: bool,
}

pub type Overrides = HashMap<u32, ElseStep>;

pub struct FrameNode {
    pub id: u32,
    pub rect: Rect,
    pub look: Look,
    pub visual: Visual,
    /// Индекс в `Frame::texts` и контентный блок текста.
    pub text: Option<(usize, Rect)>,
    pub children: Vec<FrameNode>,
    /// Какой шаг `else` применён к узлу.
    pub applied_else: Option<String>,
    /// Анимации узла в этот момент: presence поверх look, пометки для дампа.
    pub anim: Option<NodeValues>,
    anchor: Option<Value>,
}

pub struct Frame {
    pub size: [f32; 2],
    pub fps: f32,
    pub action_safe: Rect,
    pub title_safe: Rect,
    pub roots: Vec<FrameNode>,
    pub texts: Vec<TextBlock>,
    pub diagnostics: Diagnostics,
    pub fonts: Vec<LoadedFont>,
    pub system_fonts: bool,
    /// Время кадра; `None` у кадра без часов (статичная сцена).
    pub clock: Option<Clock>,
    /// Сцена снята после stop: в кадре ничего нет.
    pub removed: bool,
    /// Узлы, которые ушли из кадра, и почему.
    pub gone: std::collections::BTreeMap<u32, crate::motion::Gone>,
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

    pub fn stage_rect(&self) -> Rect {
        Rect { x: 0.0, y: 0.0, w: self.size[0], h: self.size[1] }
    }

    /// Безопасная зона по имени: `actionSafe`, `titleSafe`.
    pub fn safe_area(&self, name: &str) -> Option<Rect> {
        match name {
            "actionSafe" => Some(self.action_safe),
            "titleSafe" => Some(self.title_safe),
            _ => None,
        }
    }

    /// Прямоугольник цели якоря или `must.inside`, если она не зависит от родителя.
    pub fn named_target(&self, scene: &Scene, to: &Value) -> Option<Rect> {
        match to.as_str() {
            Some("stage") => Some(self.stage_rect()),
            Some(name) => self.safe_area(name),
            None => {
                let id = part_node(scene, to)?;
                self.find(id).map(|n| n.rect)
            }
        }
    }
}

/// Узел, на который указывает ссылка на часть `{ $part, inst }`.
pub fn part_node(scene: &Scene, to: &Value) -> Option<u32> {
    let part = to.get("$part")?.as_str()?;
    let inst = to.get("inst")?.as_str()?;
    scene.nodes.values().find(|n| n.part() == Some((inst, part))).map(|n| n.id)
}

struct Pending {
    id: u32,
    tnode: NodeId,
    look: Look,
    visual: Visual,
    text: Option<usize>,
    children: Vec<Pending>,
    /// Якорь узла, вынесенного из потока (position: absolute).
    anchor: Option<Value>,
    applied_else: Option<String>,
    anim: Option<NodeValues>,
}

struct Ctx<'a> {
    scene: &'a Scene,
    engine: &'a mut TextEngine,
    overrides: &'a Overrides,
    anim: &'a Animated,
}

pub fn build_frame_with(scene: &Scene, engine: &mut TextEngine, overrides: &Overrides, anim: &Animated) -> Result<Frame> {
    let Some(stage) = scene.stage.clone() else {
        bail!("в сцене нет stage: бандл должен вызвать mount(scene)");
    };
    let stage_rect = Rect { x: 0.0, y: 0.0, w: stage.size[0], h: stage.size[1] };
    let mut diagnostics = Diagnostics::default();
    diagnostics.0.extend(engine.diagnostics.0.iter().cloned());
    diagnostics.0.extend(scene.js_diagnostics.iter().cloned());
    diagnostics.0.extend(anim.diagnostics.0.iter().cloned());
    let mut frame = Frame {
        size: stage.size,
        fps: stage.fps,
        action_safe: stage_rect.inset(stage.action),
        title_safe: stage_rect.inset(stage.title),
        roots: Vec::new(),
        texts: Vec::new(),
        diagnostics,
        fonts: engine.fonts.clone(),
        system_fonts: engine.system_fonts,
        clock: anim.clock.clone(),
        removed: anim.removed,
        gone: Default::default(),
    };

    let mut ctx = Ctx { scene, engine, overrides, anim };
    let mut taffy: TaffyTree<usize> = TaffyTree::new();

    let roots: &[u32] = if anim.removed { &[] } else { &scene.node(stage.id).children };
    for &child in roots {
        let Some(mut pending) = build_node(&mut ctx, child, None, &mut taffy, &mut frame)? else { continue };
        let anchor = scene.node(child).props.get("anchor").cloned();
        let target = match &anchor {
            Some(a) => anchor_target(a, None, &frame, scene),
            None => Some(stage_rect),
        }
        .unwrap_or(stage_rect);

        let mut style = taffy.style(pending.tnode)?.clone();
        if anchor.is_some() {
            style.max_size = Size { width: length(target.w), height: length(target.h) };
        } else if style.size.width.is_auto() && style.size.height.is_auto() {
            style.size = Size { width: length(stage_rect.w), height: length(stage_rect.h) };
        }
        taffy.set_style(pending.tnode, style)?;

        let (texts, engine) = (&mut frame.texts, &mut *ctx.engine);
        taffy.compute_layout_with_measure(pending.tnode, Size::MAX_CONTENT, |inputs, _id, node_ctx, style| {
            taffy::compute_leaf_layout(inputs, style, |_, _| 0.0, |known, available| match node_ctx {
                Some(index) => measure_text(&mut texts[*index], engine, known, available),
                None => Size::ZERO,
            })
        })?;

        let size = taffy.layout(pending.tnode)?.size;
        let origin = match &anchor {
            Some(a) => place(a, target, size.width, size.height),
            None => [0.0, 0.0],
        };
        // Корень уже размещён якорем, второй раз его двигать не нужно.
        pending.anchor = None;
        let parent = Rect { x: origin[0], y: origin[1], w: 0.0, h: 0.0 };
        let mut root = finish(&mut ctx, &taffy, &mut pending, parent, &mut frame)?;
        root.anchor = anchor;
        frame.roots.push(root);
    }

    // Якоря к частям компонентов: цель уже разложена, двигаем поддерево целиком.
    // Цели берутся из снимка до сдвигов, поэтому цепочки якорей к частям не поддерживаются.
    let mut rects = HashMap::new();
    for root in &frame.roots {
        collect_rects(root, &mut rects);
    }
    for root in &mut frame.roots {
        place_part_anchors(scene, &rects, root);
    }
    let mut overflow = Diagnostics::default();
    for root in &frame.roots {
        check_anchors(scene, &frame, root, &mut overflow);
    }
    frame.diagnostics.0.extend(overflow.0);
    Ok(frame)
}

fn build_node(ctx: &mut Ctx, id: u32, parent_layout: Option<&str>, taffy: &mut TaffyTree<usize>, frame: &mut Frame) -> Result<Option<Pending>> {
    let scene = ctx.scene;
    let node = scene.node(id);
    let anim = ctx.anim.nodes.get(&id);
    // Скрытый узел рисуется, пока идёт его out; ушедший после out не рисуется.
    let show = anim.and_then(|a| a.show);
    if show == Some(false) || (node.hidden() && show != Some(true)) {
        if let Some(g) = anim.and_then(|a| a.gone) {
            frame.gone.insert(id, g);
        }
        return Ok(None);
    }
    let over = ctx.overrides.get(&id);
    if over.is_some_and(|o| o.hide) {
        return Ok(None);
    }
    let path = scene.path(id);
    let src = node.src.as_deref();
    let diags = &mut frame.diagnostics;

    let extra = over.map(|o| o.look.as_slice()).unwrap_or(&[]);
    let look = look::resolve(scene, node, extra, diags);
    let mut visual = look::visual(&look, &path, src, diags);
    if let Some(a) = anim {
        apply_animated(&mut visual, a);
    }
    let mut style = taffy_style(scene, id, &visual, diags);
    if let Some(a) = anim {
        if let Some(V::N(w)) = a.abs.get(&Channel::Width) {
            style.size.width = length(*w as f32);
        }
        if let Some(V::N(h)) = a.abs.get(&Channel::Height) {
            style.size.height = length(*h as f32);
        }
    }

    // Якорь внутри контейнера: в stack — ячейка сетки с выравниванием, иначе узел выносится из потока.
    let mut anchor = None;
    let in_container = node.parent.is_some_and(|p| scene.node(p).kind != Kind::Stage);
    if let Some(a) = node.props.get("anchor").filter(|_| in_container) {
        let to_parent = matches!(a.get("to").and_then(Value::as_str), None | Some("parent"));
        if parent_layout == Some("stack") && to_parent {
            stack_placement(&mut style, a);
        } else {
            style.position = Position::Absolute;
            anchor = Some(a.clone());
        }
    } else if parent_layout == Some("stack") {
        style.grid_row = Line { start: line(1), end: span(1) };
        style.grid_column = Line { start: line(1), end: span(1) };
    }

    let applied_else = over.map(|o| o.label.clone());
    let anim = anim.cloned();
    match node.kind {
        Kind::Text => {
            let texts = anim.as_ref().map(|a| &a.texts);
            let mut source = text_source(scene, id, &look, over, texts, &mut frame.diagnostics);
            if let Some(color @ V::C(_)) = anim.as_ref().and_then(|a| a.abs.get(&Channel::Color)) {
                source.look.color = color.rgba();
            }
            let requested = source.look.family.clone();
            let block = ctx.engine.build(source);
            match (&requested, &block.resolved_family) {
                (Some(req), Some(res)) if !req.eq_ignore_ascii_case(res) => frame.diagnostics.push(
                    Level::Warn,
                    "font.fallback",
                    &path,
                    src,
                    format!("шрифт {req} не найден, использован {res}"),
                    Some("подключи файл шрифта в defineScene({ fonts: [...] }) или установи шрифт в систему"),
                ),
                (_, None) if !node.text.trim().is_empty() => frame.diagnostics.push(
                    Level::Error,
                    "font.missing",
                    &path,
                    src,
                    "нет ни одного шрифта для текста",
                    Some("подключи файлы в defineScene({ fonts: [...] }) или включи systemFonts"),
                ),
                _ => {}
            }
            frame.texts.push(block);
            let index = frame.texts.len() - 1;
            let tnode = taffy.new_leaf_with_context(style, index)?;
            Ok(Some(Pending { id, tnode, look, visual, text: Some(index), children: Vec::new(), anchor, applied_else, anim }))
        }
        Kind::Box | Kind::Image | Kind::Shape | Kind::Stage => {
            if matches!(node.kind, Kind::Image | Kind::Shape) {
                frame.diagnostics.push(Level::Info, "render.unsupported", &path, src, format!("{} пока не рисуется", node.kind.name()), None);
            }
            let my_layout = node.props.get("layout").and_then(|l| l.get("kind")).and_then(Value::as_str);
            let mut children = Vec::new();
            for &c in &node.children {
                if let Some(child) = build_node(ctx, c, my_layout, taffy, frame)? {
                    children.push(child);
                }
            }
            let ids: Vec<NodeId> = children.iter().map(|c| c.tnode).collect();
            let tnode = taffy.new_with_children(style, &ids)?;
            Ok(Some(Pending { id, tnode, look, visual, text: None, children, anchor, applied_else, anim }))
        }
    }
}

fn text_source(
    scene: &Scene,
    id: u32,
    look: &Look,
    over: Option<&ElseStep>,
    texts: Option<&std::collections::BTreeMap<usize, String>>,
    diags: &mut Diagnostics,
) -> TextSource {
    let node = scene.node(id);
    let path = scene.path(id);
    let src = node.src.as_deref();
    let text_look = look::text_look(look, &path, src, diags);

    let runs = node
        .runs
        .iter()
        .enumerate()
        .map(|(i, run)| {
            let style = run.look.as_ref().and_then(Value::as_array).map(|layers| {
                let span_src = run.src.as_deref().or(src);
                let span_look = look::resolve_layers(scene, layers.iter(), Target::Span, "span", &path, span_src, diags);
                look::span_style(&span_look, &path, span_src, diags)
            });
            // Кусок count()/clock() берёт текст, который ядро посчитало для этого момента.
            let text = texts.and_then(|m| m.get(&i)).cloned().unwrap_or_else(|| run.text.clone());
            RunSpec { text, style }
        })
        .collect();

    let mut fit = node.props.get("fit").map(|f| Fit {
        shrink_to: f.get("shrinkTo").and_then(Value::as_f64).map(|v| v as f32),
        lines: f.get("lines").and_then(Value::as_u64).map(|v| v as usize),
        then: match f.get("then").and_then(Value::as_str) {
            Some("ellipsis") => FitThen::Ellipsis,
            Some("clip") => FitThen::Clip,
            _ => FitThen::None,
        },
    });
    if let Some(o) = over {
        let f = fit.get_or_insert(Fit { shrink_to: None, lines: None, then: FitThen::None });
        if o.shrink_to.is_some() {
            f.shrink_to = o.shrink_to;
        }
        if o.lines.is_some() {
            f.lines = o.lines;
        }
        if o.ellipsis {
            f.then = FitThen::Ellipsis;
        }
    }
    TextSource { runs, look: text_look, fit }
}

fn measure_text(block: &mut TextBlock, engine: &mut TextEngine, known: Size<Option<f32>>, available: Size<AvailableSpace>) -> Size<f32> {
    // С fit текст умеет сжиматься и обрезаться, поэтому его min-content — ноль.
    if block.source.fit.is_some() && known.width.is_none() && available.width == AvailableSpace::MinContent {
        block.fit_to(engine, None);
        return Size { width: 0.0, height: known.height.unwrap_or(block.layout.height().ceil()) };
    }
    let max = known.width.or(match available.width {
        AvailableSpace::Definite(w) => Some(w),
        AvailableSpace::MinContent => Some(0.0),
        AvailableSpace::MaxContent => None,
    });
    block.fit_to(engine, max);
    Size {
        width: known.width.unwrap_or(block.layout.width().ceil()),
        height: known.height.unwrap_or(block.layout.height().ceil()),
    }
}

fn finish(ctx: &mut Ctx, taffy: &TaffyTree<usize>, p: &mut Pending, parent: Rect, frame: &mut Frame) -> Result<FrameNode> {
    let layout = taffy.layout(p.tnode)?;
    let mut rect = Rect { x: parent.x + layout.location.x, y: parent.y + layout.location.y, w: layout.size.width, h: layout.size.height };

    if let Some(a) = &p.anchor
        && let Some(target) = anchor_target(a, Some(parent), frame, ctx.scene) {
            let [x, y] = place(a, target, rect.w, rect.h);
            rect.x = x;
            rect.y = y;
        }

    let text = match p.text {
        Some(index) => {
            let content = content_rect(rect, &p.visual);
            let block = &mut frame.texts[index];
            block.fit_to(ctx.engine, Some(content.w));
            block.align();
            Some((index, content))
        }
        None => None,
    };

    let mut children = Vec::with_capacity(p.children.len());
    for c in &mut p.children {
        children.push(finish(ctx, taffy, c, rect, frame)?);
    }
    Ok(FrameNode {
        id: p.id,
        rect,
        look: std::mem::take(&mut p.look),
        visual: p.visual.clone(),
        text,
        children,
        applied_else: p.applied_else.clone(),
        anim: p.anim.take(),
        anchor: p.anchor.clone(),
    })
}

/// Переходы и presence по каналам, которые перекрывают `look`.
fn apply_animated(visual: &mut Visual, a: &NodeValues) {
    for (&ch, &v) in &a.abs {
        match (ch, v) {
            (Channel::Opacity, V::N(o)) => visual.opacity = (o as f32).clamp(0.0, 1.0),
            (Channel::Radius, V::N(r)) => visual.radius = [(r as f32).max(0.0); 4],
            (Channel::Background, V::C(_)) => visual.background = Some(v.rgba()),
            (Channel::X | Channel::Y | Channel::Scale | Channel::Rotate, V::N(n)) => {
                let t = visual.transform.get_or_insert_with(Transform::default);
                let n = n as f32;
                match ch {
                    Channel::X => t.x = n,
                    Channel::Y => t.y = n,
                    Channel::Scale => t.scale = [n, n],
                    _ => t.rotate = n,
                }
            }
            _ => {}
        }
    }
}

fn collect_rects(node: &FrameNode, out: &mut HashMap<u32, Rect>) {
    out.insert(node.id, node.rect);
    for c in &node.children {
        collect_rects(c, out);
    }
}

fn place_part_anchors(scene: &Scene, rects: &HashMap<u32, Rect>, node: &mut FrameNode) {
    if let Some(to) = node.anchor.as_ref().and_then(|a| a.get("to")).filter(|t| t.is_object()) {
        let target = part_node(scene, to).and_then(|id| rects.get(&id).copied());
        if let Some(target) = target {
            let [x, y] = place(node.anchor.as_ref().expect("anchor"), target, node.rect.w, node.rect.h);
            shift(node, x - node.rect.x, y - node.rect.y);
        }
    }
    for c in &mut node.children {
        place_part_anchors(scene, rects, c);
    }
}

fn shift(node: &mut FrameNode, dx: f32, dy: f32) {
    node.rect = node.rect.shifted(dx, dy);
    if let Some((_, content)) = &mut node.text {
        *content = content.shifted(dx, dy);
    }
    for c in &mut node.children {
        shift(c, dx, dy);
    }
}

/// Якорь к safe area подразумевает, что узел в ней помещается; если нет — это сразу видно в дампе.
fn check_anchors(scene: &Scene, frame: &Frame, node: &FrameNode, diags: &mut Diagnostics) {
    if let Some(to) = node.anchor.as_ref().and_then(|a| a.get("to")).and_then(Value::as_str)
        && let Some(target) = frame.safe_area(to).filter(|t| !t.contains(&node.rect))
    {
        diags.push(
            Level::Warn,
            "anchor.overflow",
            &scene.path(node.id),
            scene.node(node.id).src.as_deref(),
            format!(
                "узел {}×{} не помещается в {to} {}×{}",
                look::num(node.rect.w),
                look::num(node.rect.h),
                look::num(target.w),
                look::num(target.h)
            ),
            Some("ограничь размер (size/max) или уменьши содержимое"),
        );
    }
    for c in &node.children {
        check_anchors(scene, frame, c, diags);
    }
}

// ─── Стиль Taffy ────────────────────────────────────────────────────────────

fn track<T: FromLength + FromPercent + FromFr + TaffyAuto>(v: &Value) -> Result<T, String> {
    if let Some(n) = v.as_f64() {
        return Ok(length(n as f32));
    }
    if let Some(p) = v.get("pct").and_then(Value::as_f64) {
        return Ok(percent(p as f32 / 100.0));
    }
    if let Some(f) = v.get("fr").and_then(Value::as_f64) {
        return Ok(fr(f as f32));
    }
    match v.as_str() {
        Some("auto") => Ok(auto()),
        _ => Err(format!("дорожка сетки: ожидается число, pct(n), {{ fr: n }} или 'auto', получено {v}")),
    }
}

fn stack_placement(style: &mut Style, anchor: &Value) {
    style.grid_row = Line { start: line(1), end: span(1) };
    style.grid_column = Line { start: line(1), end: span(1) };
    let at = anchor.get("at").and_then(Value::as_str).and_then(point).unwrap_or([0.0, 0.0]);
    let align = |f: f32| {
        if f < 0.25 {
            AlignItems::START
        } else if f > 0.75 {
            AlignItems::END
        } else {
            AlignItems::CENTER
        }
    };
    style.justify_self = Some(align(at[0]));
    style.align_self = Some(align(at[1]));
    if anchor.get("offset").is_some() {
        let [dx, dy] = anchor_offset(anchor);
        style.position = Position::Relative;
        style.inset = taffy::Rect { left: length(dx), top: length(dy), right: auto(), bottom: auto() };
    }
}

fn taffy_style(scene: &Scene, id: u32, visual: &Visual, diags: &mut Diagnostics) -> Style {
    let node = scene.node(id);
    let path = scene.path(id);
    let src = node.src.as_deref();
    let mut s = Style { display: Display::Flex, flex_direction: FlexDirection::Column, ..Default::default() };

    if let Some(layout) = node.props.get("layout") {
        match layout.get("kind").and_then(Value::as_str) {
            Some("row") => s.flex_direction = FlexDirection::Row,
            Some("column") => {}
            Some("grid") => {
                s.display = Display::Grid;
                let mut tracks = |key: &str| -> Vec<_> {
                    let Some(list) = layout.get(key).and_then(Value::as_array) else { return Vec::new() };
                    list.iter()
                        .filter_map(|t| match track(t) {
                            Ok(t) => Some(t),
                            Err(e) => {
                                diags.push(Level::Error, "layout.invalid", &path, src, format!("grid.{key}: {e}"), None);
                                None
                            }
                        })
                        .collect()
                };
                s.grid_template_columns = tracks("columns");
                s.grid_template_rows = tracks("rows");
                if s.grid_template_columns.is_empty() {
                    diags.push(Level::Error, "layout.invalid", &path, src, "grid без columns", Some("grid({ columns: [{ fr: 1 }, 120] })"));
                }
            }
            Some("stack") => {
                s.display = Display::Grid;
                s.grid_template_columns = vec![auto()];
                s.grid_template_rows = vec![auto()];
            }
            Some(other) => diags.push(Level::Error, "layout.invalid", &path, src, format!("неизвестный layout {other}"), None),
            None => diags.push(Level::Error, "layout.invalid", &path, src, "layout без kind", Some("используй row(), column(), grid() или stack()")),
        }
        match layout.get("gap") {
            Some(Value::Array(g)) if g.len() == 2 => {
                let f = |i: usize| g[i].as_f64().unwrap_or(0.0) as f32;
                s.gap = Size { width: length(f(0)), height: length(f(1)) };
            }
            Some(g) => {
                if let Some(g) = g.as_f64() {
                    s.gap = Size { width: length(g as f32), height: length(g as f32) };
                }
            }
            None => {}
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

    if let Some(cell) = node.props.get("cell") {
        let n = |k: &str| cell.get(k).and_then(Value::as_i64);
        let axis = |start: Option<i64>, span_n: Option<i64>| Line {
            start: start.map_or(auto(), |v| line(v as i16)),
            end: span(span_n.unwrap_or(1).max(1) as u16),
        };
        if n("col").is_some() || n("colSpan").is_some() {
            s.grid_column = axis(n("col"), n("colSpan"));
        }
        if n("row").is_some() || n("rowSpan").is_some() {
            s.grid_row = axis(n("row"), n("rowSpan"));
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
    s.size = dims("size").map(|d| d.map_or(Dimension::auto(), SizeValue::dimension));
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

/// Цель якоря. `None` для частей компонентов: их размещает второй проход.
fn anchor_target(anchor: &Value, parent: Option<Rect>, frame: &Frame, scene: &Scene) -> Option<Rect> {
    match anchor.get("to") {
        Some(to) if to.is_object() => None,
        None => Some(parent.unwrap_or(frame.stage_rect())),
        Some(to) if to.as_str() == Some("parent") => Some(parent.unwrap_or(frame.stage_rect())),
        Some(to) => Some(frame.named_target(scene, to).unwrap_or(frame.stage_rect())),
    }
}

/// `anchor.offset` как `[dx, dy]`; недостающие значения — 0.
fn anchor_offset(anchor: &Value) -> [f32; 2] {
    let o = anchor.get("offset").and_then(Value::as_array);
    let f = |i: usize| o.and_then(|o| o.get(i)).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    [f(0), f(1)]
}

fn place(anchor: &Value, target: Rect, w: f32, h: f32) -> [f32; 2] {
    let at = anchor.get("at").and_then(Value::as_str).and_then(point).unwrap_or([0.0, 0.0]);
    let selfp = anchor.get("self").and_then(Value::as_str).and_then(point).unwrap_or(at);
    let offset = anchor_offset(anchor);
    [
        target.x + at[0] * target.w - selfp[0] * w + offset[0],
        target.y + at[1] * target.h - selfp[1] * h + offset[1],
    ]
}
