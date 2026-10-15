//! Утверждения `must` и деградация `else`.
//!
//! В dev нарушение — ошибка, а в подсказке сказано, какой шаг `else` его исправил бы.
//! В эфире (air) шаги перебираются по порядку и применяется первый, при котором `must`
//! выполнен; если ни один не помог — последний. Шаги независимые, как варианты ViewThatFits:
//! `else[k]` применяется сам по себе, без шагов перед ним.

use anyhow::Result;
use serde_json::Value;

use crate::diag::{Diagnostics, Level};
use crate::layout::{ElseStep, Frame, FrameNode, Overrides, Rect, build_frame, build_frame_with, content_rect};
use crate::look::{Rgba, num};
use crate::scene::{Kind, Scene};
use crate::text::TextEngine;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Разработка и тесты: нарушения — ошибки, кадр рисуется как есть.
    Dev,
    /// Эфир: нарушения исправляются шагами `else`, кадр не падает.
    Air,
}

#[derive(Debug, Clone, Copy)]
pub struct EvalOptions {
    pub mode: Mode,
    /// Подложка под сценой; без неё контраст над прозрачностью не проверить.
    pub background: Option<Rgba>,
}

impl Default for EvalOptions {
    fn default() -> Self {
        Self { mode: Mode::Dev, background: None }
    }
}

#[derive(Debug, Clone)]
pub struct Violation {
    pub id: u32,
    pub rule: &'static str,
    pub message: String,
}

/// Строит кадр, проверяет `must` и в режиме air применяет `else`.
pub fn evaluate(scene: &Scene, engine: &mut TextEngine, opts: EvalOptions) -> Result<Frame> {
    let base = build_frame(scene, engine)?;
    let (violations, notes) = check(scene, &base, opts);
    if violations.is_empty() {
        let mut frame = base;
        frame.diagnostics.0.extend(notes.0);
        return Ok(frame);
    }

    match opts.mode {
        Mode::Dev => {
            let mut frame = base;
            for v in &violations {
                let steps = else_steps(scene, v.id);
                let hint = if steps.is_empty() {
                    Some(default_hint(v.rule).to_string())
                } else {
                    match first_fix(scene, engine, opts, v.id, &Overrides::new(), &steps)? {
                        Some(step) => Some(format!("{} это исправляет; в эфире шаг применится сам", step.label)),
                        None => Some(format!("ни один шаг else ({} шт.) это не исправляет; {}", steps.len(), default_hint(v.rule))),
                    }
                };
                push(&mut frame.diagnostics, scene, v, Level::Error, hint);
            }
            frame.diagnostics.0.extend(notes.0);
            Ok(frame)
        }
        Mode::Air => {
            let mut overrides = Overrides::new();
            let mut ids: Vec<u32> = violations.iter().map(|v| v.id).collect();
            ids.dedup();
            for id in ids {
                let steps = else_steps(scene, id);
                if steps.is_empty() {
                    continue;
                }
                let chosen = first_fix(scene, engine, opts, id, &overrides, &steps)?.unwrap_or_else(|| steps.last().expect("non-empty").clone());
                overrides.insert(id, chosen);
            }
            let mut frame = build_frame_with(scene, engine, &overrides)?;
            let (remaining, notes) = check(scene, &frame, opts);
            for v in &violations {
                if let Some(step) = overrides.get(&v.id) {
                    let fixed = !remaining.iter().any(|r| r.id == v.id && r.rule == v.rule);
                    let msg = Violation {
                        message: format!("{}; применён {}{}", v.message, step.label, if fixed { "" } else { ", не помог" }),
                        ..v.clone()
                    };
                    push(&mut frame.diagnostics, scene, &msg, Level::Warn, None);
                }
            }
            for v in &remaining {
                push(&mut frame.diagnostics, scene, v, Level::Error, Some(default_hint(v.rule).to_string()));
            }
            frame.diagnostics.0.extend(notes.0);
            Ok(frame)
        }
    }
}

fn push(diags: &mut Diagnostics, scene: &Scene, v: &Violation, level: Level, hint: Option<String>) {
    diags.push(level, &format!("must.{}", v.rule), &scene.path(v.id), scene.node(v.id).src.as_deref(), v.message.clone(), hint.as_deref());
}

fn default_hint(rule: &str) -> &'static str {
    match rule {
        "fit" => "добавь fit={{ shrinkTo }} или else=[{ shrinkTo }, { lines: 2 }, { ellipsis: true }], либо дай узлу больше места",
        "inside" => "ограничь размер (size/max) или поменяй якорь",
        "contrast" => "сделай текст светлее/темнее или добавь подложку под текст",
        "visible" => "у узла нулевой размер, нулевая прозрачность или он целиком за кадром",
        _ => "",
    }
}

/// Шаги `else` узла в порядке перебора; каждый — самостоятельный вариант.
pub fn else_steps(scene: &Scene, id: u32) -> Vec<ElseStep> {
    let Some(raw) = scene.node(id).props.get("else").and_then(Value::as_array) else { return Vec::new() };
    raw.iter()
        .enumerate()
        .map(|(k, step)| ElseStep {
            label: format!("else[{k}]"),
            shrink_to: step.get("shrinkTo").and_then(Value::as_f64).map(|v| v as f32),
            lines: step.get("lines").and_then(Value::as_u64).map(|v| v as usize),
            ellipsis: step.get("ellipsis").and_then(Value::as_bool) == Some(true),
            look: step.get("look").and_then(Value::as_array).cloned().unwrap_or_default(),
            hide: step.get("hide").and_then(Value::as_bool) == Some(true),
        })
        .collect()
}

fn first_fix(scene: &Scene, engine: &mut TextEngine, opts: EvalOptions, id: u32, base: &Overrides, steps: &[ElseStep]) -> Result<Option<ElseStep>> {
    for step in steps {
        let mut overrides = base.clone();
        overrides.insert(id, step.clone());
        let frame = build_frame_with(scene, engine, &overrides)?;
        if node_violations(scene, &frame, opts, id, &mut Diagnostics::default()).is_empty() {
            return Ok(Some(step.clone()));
        }
    }
    Ok(None)
}

/// Все нарушения в кадре и информационные заметки (например, контраст не проверить).
pub fn check(scene: &Scene, frame: &Frame, opts: EvalOptions) -> (Vec<Violation>, Diagnostics) {
    let mut notes = Diagnostics::default();
    let mut out = Vec::new();
    for node in scene.nodes.values() {
        if node.props.contains_key("must") {
            out.extend(node_violations(scene, frame, opts, node.id, &mut notes));
        }
    }
    (out, notes)
}

fn node_violations(scene: &Scene, frame: &Frame, opts: EvalOptions, id: u32, notes: &mut Diagnostics) -> Vec<Violation> {
    let node = scene.node(id);
    let Some(must) = node.props.get("must") else { return Vec::new() };
    // Узел скрыт (when=false или else с hide): проверять нечего.
    let Some(f) = frame.find(id) else { return Vec::new() };
    let mut out = Vec::new();
    let mut fail = |rule: &'static str, message: String| out.push(Violation { id, rule, message });

    if must.get("fit").and_then(Value::as_bool) == Some(true)
        && let Some(msg) = fit_problem(frame, f) {
            fail("fit", msg);
        }
    if let Some(to) = must.get("inside") {
        let target = match to.as_str() {
            Some("parent") => node.parent.and_then(|p| frame.find(p)).map(|p| p.rect),
            _ => frame.named_target(scene, to),
        };
        match target {
            Some(t) if !t.contains(&f.rect) => fail("inside", format!("{} выходит за {} {}", rect(&f.rect), target_name(to), rect(&t))),
            Some(_) => {}
            None => notes.push(Level::Warn, "must.inside", &scene.path(id), node.src.as_deref(), format!("цель {to} не найдена"), None),
        }
    }
    if let Some(min) = must.get("contrast").and_then(Value::as_f64) {
        match contrast(scene, frame, id, opts.background) {
            Some(ratio) if ratio + 1e-3 < min as f32 => fail("contrast", format!("контраст {} ниже {}", num(ratio), num(min as f32))),
            Some(_) => {}
            None => notes.push(
                Level::Info,
                "must.contrast",
                &scene.path(id),
                node.src.as_deref(),
                "контраст не проверить: под текстом прозрачность",
                Some("передай подложку эфира через --bg #RRGGBB"),
            ),
        }
    }
    if must.get("visible").and_then(Value::as_bool) == Some(true) {
        let opacity = chain(scene, id).iter().filter_map(|i| frame.find(*i)).map(|n| n.visual.opacity).product::<f32>();
        if f.rect.w <= 0.0 || f.rect.h <= 0.0 {
            fail("visible", format!("нулевой размер {}", rect(&f.rect)));
        } else if !frame.stage_rect().intersects(&f.rect) {
            fail("visible", format!("{} целиком за кадром", rect(&f.rect)));
        } else if opacity <= 0.0 {
            fail("visible", "итоговая прозрачность 0".into());
        }
    }
    out
}

fn rect(r: &Rect) -> String {
    format!("({},{} {}×{})", num(r.x), num(r.y), num(r.w), num(r.h))
}

fn target_name(to: &Value) -> String {
    match to.as_str() {
        Some(s) => s.to_string(),
        None => format!(
            "{}/{}",
            to.get("inst").and_then(Value::as_str).unwrap_or("?"),
            to.get("$part").and_then(Value::as_str).unwrap_or("?")
        ),
    }
}

fn fit_problem(frame: &Frame, f: &FrameNode) -> Option<String> {
    let content = content_rect(f.rect, &f.visual);
    if let Some((index, text_box)) = f.text {
        let block = &frame.texts[index];
        if block.clip {
            return None;
        }
        let (w, h) = (block.layout.width(), block.layout.height());
        if block.overflow {
            if let Some(fit) = &block.source.fit {
                let allowed = fit.lines.unwrap_or(1);
                if block.line_count() > allowed {
                    return Some(format!(
                        "даже при кегле {} текст занимает {} строк(и) при ширине {}, разрешено {allowed}",
                        num(block.size),
                        block.line_count(),
                        num(text_box.w)
                    ));
                }
            }
            return Some(format!("слово шире блока: текст {} при ширине {}", num(w), num(text_box.w)));
        }
        if w > text_box.w + 0.5 || h > text_box.h + 0.5 {
            return Some(format!("текст {}×{} не помещается в {}×{}", num(w), num(h), num(text_box.w), num(text_box.h)));
        }
        return None;
    }
    let outside = f.children.iter().find(|c| !content.contains(&c.rect))?;
    Some(format!("дочерний узел {} выходит за контентный блок {}", rect(&outside.rect), rect(&content)))
}

/// Цепочка от корня сцены до узла включительно.
fn chain(scene: &Scene, id: u32) -> Vec<u32> {
    let mut ids = vec![id];
    let mut cur = scene.node(id).parent;
    while let Some(p) = cur {
        if scene.node(p).kind == Kind::Stage {
            break;
        }
        ids.push(p);
        cur = scene.node(p).parent;
    }
    ids.reverse();
    ids
}

fn over(src: Rgba, dst: Rgba) -> Rgba {
    let a = src.a + dst.a * (1.0 - src.a);
    if a <= 0.0 {
        return Rgba::default();
    }
    let mix = |s: f32, d: f32| (s * src.a + d * dst.a * (1.0 - src.a)) / a;
    Rgba { r: mix(src.r, dst.r), g: mix(src.g, dst.g), b: mix(src.b, dst.b), a }
}

fn luminance(c: Rgba) -> f32 {
    let lin = |v: f32| if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

/// WCAG-контраст самого слабого цвета текста к фону под ним. Фон считается по подложкам
/// предков (с учётом прозрачности), а не по пикселям; `None`, если под текстом прозрачность.
fn contrast(scene: &Scene, frame: &Frame, id: u32, background: Option<Rgba>) -> Option<f32> {
    let f = frame.find(id)?;
    let (index, _) = f.text?;
    let mut bg = background.unwrap_or_default();
    let mut opacity = 1.0;
    for i in chain(scene, id) {
        let Some(n) = frame.find(i) else { continue };
        opacity *= n.visual.opacity;
        if let Some(c) = n.visual.background {
            bg = over(Rgba { a: c.a * opacity, ..c }, bg);
        }
    }
    if bg.a < 0.99 {
        return None;
    }
    frame.texts[index]
        .colors()
        .into_iter()
        .map(|c| {
            let text = over(Rgba { a: c.a * opacity, ..c }, bg);
            let (l1, l2) = (luminance(text), luminance(bg));
            (l1.max(l2) + 0.05) / (l1.min(l2) + 0.05)
        })
        .reduce(f32::min)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn else_variants_are_independent() {
        let mut scene = Scene::default();
        scene
            .apply_batch(r#"[{"op":"create","id":1,"kind":"text"},{"op":"set","id":1,"key":"else","value":[{"shrinkTo":28},{"lines":2}]}]"#)
            .unwrap();
        let steps = else_steps(&scene, 1);
        assert_eq!(steps.len(), 2);
        assert_eq!((steps[0].label.as_str(), steps[0].shrink_to, steps[0].lines), ("else[0]", Some(28.0), None));
        // Второй вариант не наследует shrinkTo первого.
        assert_eq!((steps[1].label.as_str(), steps[1].shrink_to, steps[1].lines), ("else[1]", None, Some(2)));
    }

    #[test]
    fn wcag_contrast_extremes() {
        let black = Rgba { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
        let l = (luminance(Rgba::WHITE) + 0.05) / (luminance(black) + 0.05);
        assert!((l - 21.0).abs() < 0.01, "{l}");
    }

    #[test]
    fn over_is_porter_duff() {
        let half_white = Rgba { a: 0.5, ..Rgba::WHITE };
        let black = Rgba { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
        let c = over(half_white, black);
        assert!((c.r - 0.5).abs() < 1e-6 && c.a == 1.0);
    }
}
