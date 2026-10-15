//! Рендер кадра в пиксели через vello_cpu (headless, без GPU).

use anyhow::{Result, bail};
use kine_core::look::{Rgba, Visual};
use kine_core::{Frame, FrameNode, Rect};
use parley::PositionedLayoutItem;
use vello_cpu::color::AlphaColor;
use vello_cpu::color::Srgb;
use vello_cpu::kurbo::{self, Affine, Shape as _};
use vello_cpu::{Glyph, Pixmap, RenderContext, Resources};

pub struct RenderOptions {
    /// Масштаб опорного кадра к выходу: 1 — 1920×1080, 2 — 4K.
    pub scale: f32,
    /// Подложка; без неё кадр с прозрачностью (fill + key).
    pub background: Option<Rgba>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self { scale: 1.0, background: None }
    }
}

fn color(c: Rgba) -> AlphaColor<Srgb> {
    AlphaColor::new([c.r, c.g, c.b, c.a])
}

fn kurbo_rect(r: Rect) -> kurbo::Rect {
    kurbo::Rect::new(r.x as f64, r.y as f64, (r.x + r.w) as f64, (r.y + r.h) as f64)
}

fn rounded(r: Rect, radii: [f32; 4]) -> kurbo::BezPath {
    let [tl, tr, br, bl] = radii.map(f64::from);
    kurbo::RoundedRect::from_rect(kurbo_rect(r), kurbo::RoundedRectRadii::new(tl, tr, br, bl)).to_path(0.1)
}

pub fn render(frame: &Frame, opts: &RenderOptions) -> Result<Pixmap> {
    let w = (frame.size[0] * opts.scale).round();
    let h = (frame.size[1] * opts.scale).round();
    if w < 1.0 || h < 1.0 || w > u16::MAX as f32 || h > u16::MAX as f32 {
        bail!("размер выхода {w}×{h} вне допустимого диапазона");
    }
    let (w, h) = (w as u16, h as u16);
    let mut ctx = RenderContext::new(w, h);
    let mut res = Resources::new();
    let base = Affine::scale(opts.scale as f64);

    if let Some(bg) = opts.background {
        ctx.set_paint(color(bg));
        ctx.fill_rect(&kurbo::Rect::new(0.0, 0.0, w as f64, h as f64));
    }
    for root in &frame.roots {
        draw(&mut ctx, &mut res, frame, root, base);
    }

    let mut pixmap = Pixmap::new(w, h);
    ctx.flush();
    ctx.render(&mut pixmap, &mut res);
    Ok(pixmap)
}

pub fn render_png(frame: &Frame, opts: &RenderOptions) -> Result<Vec<u8>> {
    Ok(render(frame, opts)?.into_png()?)
}

fn node_transform(v: &Visual, r: Rect) -> Affine {
    let Some(t) = &v.transform else { return Affine::IDENTITY };
    let ox = (r.x + r.w * t.origin[0]) as f64;
    let oy = (r.y + r.h * t.origin[1]) as f64;
    Affine::translate((t.x as f64, t.y as f64))
        * Affine::translate((ox, oy))
        * Affine::rotate((t.rotate as f64).to_radians())
        * Affine::scale_non_uniform(t.scale[0] as f64, t.scale[1] as f64)
        * Affine::translate((-ox, -oy))
}

fn draw(ctx: &mut RenderContext, res: &mut Resources, frame: &Frame, node: &FrameNode, parent: Affine) {
    let v = &node.visual;
    let r = node.rect;
    let tr = parent * node_transform(v, r);
    ctx.set_transform(tr);

    let layered = v.opacity < 1.0 || v.clip;
    if layered {
        let clip = v.clip.then(|| rounded(r, v.radius));
        ctx.push_layer(clip.as_ref(), None, (v.opacity < 1.0).then_some(v.opacity), None, None);
    }

    for s in v.shadows.iter().filter(|s| !s.inset) {
        let sr = Rect { x: r.x + s.x - s.spread, y: r.y + s.y - s.spread, w: r.w + 2.0 * s.spread, h: r.h + 2.0 * s.spread };
        ctx.set_paint(color(s.color));
        let radius = v.radius.iter().copied().fold(0.0, f32::max);
        ctx.fill_blurred_rounded_rect(&kurbo_rect(sr), radius, s.blur / 2.0, false);
    }
    if let Some(bg) = v.background {
        ctx.set_paint(color(bg));
        ctx.fill_path(&rounded(r, v.radius));
    }
    if let Some(b) = &v.border {
        // Обводка внутри узла: сдвигаем контур на половину толщины.
        let half = b.width / 2.0;
        let inner = Rect { x: r.x + half, y: r.y + half, w: (r.w - b.width).max(0.0), h: (r.h - b.width).max(0.0) };
        ctx.set_stroke(kurbo::Stroke::new(b.width as f64));
        ctx.set_paint(color(b.color));
        ctx.stroke_path(&rounded(inner, v.radius.map(|x| (x - half).max(0.0))));
    }

    if let Some((index, origin)) = node.text {
        draw_text(ctx, res, frame, index, origin, tr);
    }
    for child in &node.children {
        draw(ctx, res, frame, child, tr);
    }

    if layered {
        ctx.set_transform(tr);
        ctx.pop_layer();
    }
}

fn draw_text(ctx: &mut RenderContext, res: &mut Resources, frame: &Frame, index: usize, origin: [f32; 2], tr: Affine) {
    let block = &frame.texts[index];
    ctx.set_transform(tr * Affine::translate((origin[0] as f64, origin[1] as f64)));
    for line in block.layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(glyph_run) = item else { continue };
            let [r, g, b, a] = glyph_run.style().brush;
            ctx.set_paint(AlphaColor::<Srgb>::from_rgba8(r, g, b, a));
            let run = glyph_run.run();
            let glyphs = glyph_run.positioned_glyphs().map(|g| Glyph { id: g.id, x: g.x, y: g.y });
            // parley и glifo описывают координаты вариативного шрифта разными типами поверх i16.
            let coords: Vec<i16> = run.normalized_coords().iter().map(|c| c.to_bits()).collect();
            // Ошибка отдельного глифа не роняет кадр: остальные глифы уже нарисованы.
            let _ = ctx
                .glyph_run(res, run.font())
                .font_size(run.font_size())
                .normalized_coords(&coords)
                .hint(false)
                .fill_glyphs(glyphs);
        }
    }
}
