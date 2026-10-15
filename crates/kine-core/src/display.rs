//! Display list: плоский список команд рисования между кадром ядра и бэкендом.
//!
//! Ядро строит его из кадра один раз: стили, раскладка, текст и движение здесь уже
//! превращены в прямоугольники, тени, прогоны глифов и слои. Бэкенд (vello_cpu сейчас,
//! GPU в M8) только исполняет команды и ничего не знает о дереве сцены.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use parley::{FontData, PositionedLayoutItem};

use crate::layout::{Frame, FrameNode, Rect};
use crate::look::{Rgba, Visual};
use crate::motion::RelMotion;
use crate::text::TextBlock;

/// Аффинное преобразование в порядке kurbo: `x' = a·x + c·y + e`, `y' = b·x + d·y + f`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine(pub [f64; 6]);

impl Affine {
    pub const IDENTITY: Affine = Affine([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    pub fn translate(x: f64, y: f64) -> Affine {
        Affine([1.0, 0.0, 0.0, 1.0, x, y])
    }

    pub fn scale(sx: f64, sy: f64) -> Affine {
        Affine([sx, 0.0, 0.0, sy, 0.0, 0.0])
    }

    pub fn rotate(radians: f64) -> Affine {
        let (s, c) = radians.sin_cos();
        Affine([c, s, -s, c, 0.0, 0.0])
    }
}

impl std::ops::Mul for Affine {
    type Output = Affine;

    /// `self * other`: сначала `other`, затем `self`.
    fn mul(self, o: Affine) -> Affine {
        let [a, b, c, d, e, f] = self.0;
        let [a2, b2, c2, d2, e2, f2] = o.0;
        Affine([a * a2 + c * b2, b * a2 + d * b2, a * c2 + c * d2, b * c2 + d * d2, a * e2 + c * f2 + e, b * e2 + d * f2 + f])
    }
}

#[derive(Debug, Clone)]
pub struct GlyphRun {
    pub font: FontData,
    pub size: f32,
    /// Координаты вариативного шрифта.
    pub coords: Vec<i16>,
    pub color: [u8; 4],
    /// (id, x, y) относительно начала текста.
    pub glyphs: Vec<(u32, f32, f32)>,
}

#[derive(Debug, Clone)]
pub enum Item {
    /// Слой с прозрачностью и/или клипом по скруглённому прямоугольнику; закрывается `Pop`.
    PushLayer { transform: Affine, clip: Option<(Rect, [f32; 4])>, opacity: f32 },
    /// Клип по прямоугольнику; закрывается `Pop`.
    PushClip { transform: Affine, rect: Rect },
    Pop,
    Fill { transform: Affine, rect: Rect, radii: [f32; 4], color: Rgba },
    /// Обводка по контуру `rect` толщиной `width` (контур уже сдвинут внутрь узла).
    Stroke { transform: Affine, rect: Rect, radii: [f32; 4], width: f32, color: Rgba },
    /// Размытая тень скруглённого прямоугольника.
    Shadow { transform: Affine, rect: Rect, radius: f32, std_dev: f32, color: Rgba },
    Glyphs { transform: Affine, run: GlyphRun },
}

impl Item {
    fn kind(&self) -> &'static str {
        match self {
            Item::PushLayer { .. } => "layer",
            Item::PushClip { .. } => "clip",
            Item::Pop => "pop",
            Item::Fill { .. } => "fill",
            Item::Stroke { .. } => "stroke",
            Item::Shadow { .. } => "shadow",
            Item::Glyphs { .. } => "glyphs",
        }
    }

    fn transformed(&self, by: Affine) -> Item {
        let mut item = self.clone();
        match &mut item {
            Item::PushLayer { transform, .. }
            | Item::PushClip { transform, .. }
            | Item::Fill { transform, .. }
            | Item::Stroke { transform, .. }
            | Item::Shadow { transform, .. }
            | Item::Glyphs { transform, .. } => *transform = by * *transform,
            Item::Pop => {}
        }
        item
    }
}

#[derive(Debug, Clone)]
pub struct DisplayList {
    /// Размер в единицах опорного кадра; бэкенд масштабирует его к выходу.
    pub size: [f32; 2],
    pub items: Vec<Item>,
}

impl DisplayList {
    pub fn new(size: [f32; 2]) -> DisplayList {
        DisplayList { size, items: Vec::new() }
    }

    pub fn from_frame(frame: &Frame) -> DisplayList {
        let mut dl = DisplayList::new(frame.size);
        for root in &frame.roots {
            dl.node(frame, root, Affine::IDENTITY);
        }
        dl
    }

    /// Вставляет чужой список с преобразованием (раскадровка, композиция).
    pub fn append(&mut self, other: &DisplayList, transform: Affine) {
        self.items.extend(other.items.iter().map(|i| i.transformed(transform)));
    }

    /// Счётчики команд: `fill 4 · glyphs 2 · layer 1`.
    pub fn summary(&self) -> String {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for item in &self.items {
            *counts.entry(item.kind()).or_default() += 1;
        }
        counts.iter().map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join(" · ")
    }

    fn node(&mut self, frame: &Frame, node: &FrameNode, parent: Affine) {
        let v = &node.visual;
        let r = node.rect;
        let motion = node.anim.as_ref().map(|a| a.motion).unwrap_or_default();
        let tr = parent * node_transform(v, &motion, r);

        let opacity = (v.opacity * motion.opacity).clamp(0.0, 1.0);
        if opacity <= 0.0 {
            return;
        }
        let layered = opacity < 1.0 || v.clip;
        if layered {
            let clip = v.clip.then_some((r, v.radius));
            self.items.push(Item::PushLayer { transform: tr, clip, opacity });
        }

        let radius = v.radius.iter().copied().fold(0.0, f32::max);
        for s in v.shadows.iter().filter(|s| !s.inset) {
            let rect = Rect { x: r.x + s.x - s.spread, y: r.y + s.y - s.spread, w: r.w + 2.0 * s.spread, h: r.h + 2.0 * s.spread };
            self.items.push(Item::Shadow { transform: tr, rect, radius, std_dev: s.blur / 2.0, color: s.color });
        }
        if let Some(bg) = v.background {
            self.items.push(Item::Fill { transform: tr, rect: r, radii: v.radius, color: bg });
        }
        if let Some(b) = &v.border {
            // Обводка внутри узла: контур сдвинут на половину толщины.
            let half = b.width / 2.0;
            let rect = Rect { x: r.x + half, y: r.y + half, w: (r.w - b.width).max(0.0), h: (r.h - b.width).max(0.0) };
            self.items.push(Item::Stroke { transform: tr, rect, radii: v.radius.map(|x| (x - half).max(0.0)), width: b.width, color: b.color });
        }

        if let Some((index, content)) = node.text {
            let block = &frame.texts[index];
            if block.clip {
                self.items.push(Item::PushClip { transform: tr, rect: content });
            }
            self.text(block, [content.x, content.y], tr, motion.reveal);
            if block.clip {
                self.items.push(Item::Pop);
            }
        }
        for child in &node.children {
            self.node(frame, child, tr);
        }
        if layered {
            self.items.push(Item::Pop);
        }
    }

    /// Прогоны глифов текста. `reveal` — доля видимых глифов (эффект печати).
    pub fn text(&mut self, block: &TextBlock, origin: [f32; 2], transform: Affine, reveal: f32) {
        let tr = transform * Affine::translate(origin[0] as f64, origin[1] as f64);
        let total: usize = block
            .layout
            .lines()
            .flat_map(|l| l.items())
            .map(|i| match i {
                PositionedLayoutItem::GlyphRun(g) => g.glyphs().count(),
                _ => 0,
            })
            .sum();
        let mut budget = if reveal >= 1.0 { usize::MAX } else { (reveal.max(0.0) * total as f32).round() as usize };
        for line in block.layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(glyph_run) = item else { continue };
                if budget == 0 {
                    return;
                }
                let run = glyph_run.run();
                let glyphs: Vec<(u32, f32, f32)> = glyph_run.positioned_glyphs().take(budget).map(|g| (g.id, g.x, g.y)).collect();
                budget = budget.saturating_sub(glyphs.len());
                self.items.push(Item::Glyphs {
                    transform: tr,
                    run: GlyphRun {
                        font: run.font().clone(),
                        size: run.font_size(),
                        // parley и glifo описывают координаты вариативного шрифта разными типами поверх i16.
                        coords: run.normalized_coords().iter().map(|c| c.to_bits()).collect(),
                        color: glyph_run.style().brush,
                        glyphs,
                    },
                });
            }
        }
    }

    /// Текстовое описание для отладки: по строке на команду.
    pub fn describe(&self) -> String {
        let mut out = String::new();
        for item in &self.items {
            let _ = match item {
                Item::Fill { rect, color, .. } => writeln!(out, "fill {} {color}", rect_str(rect)),
                Item::Stroke { rect, width, color, .. } => writeln!(out, "stroke {} w{width} {color}", rect_str(rect)),
                Item::Shadow { rect, std_dev, color, .. } => writeln!(out, "shadow {} σ{std_dev} {color}", rect_str(rect)),
                Item::Glyphs { run, .. } => writeln!(out, "glyphs ×{} {}px", run.glyphs.len(), run.size),
                Item::PushLayer { opacity, clip, .. } => writeln!(out, "layer opacity {opacity}{}", if clip.is_some() { " clip" } else { "" }),
                Item::PushClip { rect, .. } => writeln!(out, "clip {}", rect_str(rect)),
                Item::Pop => writeln!(out, "pop"),
            };
        }
        out
    }
}

fn rect_str(r: &Rect) -> String {
    format!("{},{} {}×{}", r.x, r.y, r.w, r.h)
}

/// Трансформация узла: `transform` из look плюс слой presence (сдвиг, поворот, масштаб).
fn node_transform(v: &Visual, m: &RelMotion, r: Rect) -> Affine {
    if v.transform.is_none() && m.dx == 0.0 && m.dy == 0.0 && m.scale == 1.0 && m.rotate == 0.0 {
        return Affine::IDENTITY;
    }
    let (x, y, scale, rotate, origin) = match &v.transform {
        Some(t) => (t.x, t.y, t.scale, t.rotate, t.origin),
        None => (0.0, 0.0, [1.0, 1.0], 0.0, [0.5, 0.5]),
    };
    let ox = (r.x + r.w * origin[0]) as f64;
    let oy = (r.y + r.h * origin[1]) as f64;
    Affine::translate((x + m.dx) as f64, (y + m.dy) as f64)
        * Affine::translate(ox, oy)
        * Affine::rotate(((rotate + m.rotate) as f64).to_radians())
        * Affine::scale((scale[0] * m.scale) as f64, (scale[1] * m.scale) as f64)
        * Affine::translate(-ox, -oy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affine_composition_order() {
        // Сначала масштаб, потом сдвиг.
        let a = Affine::translate(10.0, 0.0) * Affine::scale(2.0, 2.0);
        let [aa, _, _, _, e, _] = a.0;
        assert_eq!((aa, e), (2.0, 10.0));
        let p = |m: Affine, x: f64, y: f64| (m.0[0] * x + m.0[2] * y + m.0[4], m.0[1] * x + m.0[3] * y + m.0[5]);
        assert_eq!(p(a, 1.0, 1.0), (12.0, 2.0));
        let r = Affine::rotate(std::f64::consts::FRAC_PI_2);
        let (x, y) = p(r, 1.0, 0.0);
        assert!(x.abs() < 1e-12 && (y - 1.0).abs() < 1e-12);
    }
}
