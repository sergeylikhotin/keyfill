//! CPU-бэкенд: исполняет display list через vello_cpu (headless, без GPU).
//!
//! Это эталонный исполнитель: на нём золотые тесты и рендер в файл с побайтной
//! воспроизводимостью. О дереве сцены, стилях и тексте бэкенд ничего не знает.

use anyhow::{Result, bail};
use kine_core::display::{Affine, DisplayList, Item};
use kine_core::look::Rgba;
use kine_core::{Frame, Rect};
use vello_cpu::color::AlphaColor;
use vello_cpu::color::Srgb;
use vello_cpu::kurbo::{self, Shape as _};
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

fn affine(a: Affine) -> kurbo::Affine {
    kurbo::Affine::new(a.0)
}

/// Рендер кадра: строит display list и исполняет его.
pub fn render(frame: &Frame, opts: &RenderOptions) -> Result<Pixmap> {
    execute(&DisplayList::from_frame(frame), opts)
}

pub fn render_png(frame: &Frame, opts: &RenderOptions) -> Result<Vec<u8>> {
    Ok(render(frame, opts)?.into_png()?)
}

pub fn execute(dl: &DisplayList, opts: &RenderOptions) -> Result<Pixmap> {
    let w = (dl.size[0] * opts.scale).round();
    let h = (dl.size[1] * opts.scale).round();
    if w < 1.0 || h < 1.0 || w > u16::MAX as f32 || h > u16::MAX as f32 {
        bail!("размер выхода {w}×{h} вне допустимого диапазона");
    }
    let (w, h) = (w as u16, h as u16);
    let mut ctx = RenderContext::new(w, h);
    let mut res = Resources::new();
    let base = kurbo::Affine::scale(opts.scale as f64);

    if let Some(bg) = opts.background {
        ctx.set_paint(color(bg));
        ctx.fill_rect(&kurbo::Rect::new(0.0, 0.0, w as f64, h as f64));
    }
    for item in &dl.items {
        match item {
            Item::PushLayer { transform, clip, opacity } => {
                ctx.set_transform(base * affine(*transform));
                let clip = clip.map(|(r, radii)| rounded(r, radii));
                ctx.push_layer(clip.as_ref(), None, (*opacity < 1.0).then_some(*opacity), None, None);
            }
            Item::PushClip { transform, rect } => {
                ctx.set_transform(base * affine(*transform));
                ctx.push_clip_layer(&kurbo_rect(*rect).to_path(0.1));
            }
            Item::Pop => ctx.pop_layer(),
            Item::Fill { transform, rect, radii, color: c } => {
                ctx.set_transform(base * affine(*transform));
                ctx.set_paint(color(*c));
                ctx.fill_path(&rounded(*rect, *radii));
            }
            Item::Stroke { transform, rect, radii, width, color: c } => {
                ctx.set_transform(base * affine(*transform));
                ctx.set_stroke(kurbo::Stroke::new(*width as f64));
                ctx.set_paint(color(*c));
                ctx.stroke_path(&rounded(*rect, *radii));
            }
            Item::Shadow { transform, rect, radius, std_dev, color: c } => {
                ctx.set_transform(base * affine(*transform));
                ctx.set_paint(color(*c));
                ctx.fill_blurred_rounded_rect(&kurbo_rect(*rect), *radius, *std_dev, false);
            }
            Item::Glyphs { transform, run } => {
                ctx.set_transform(base * affine(*transform));
                let [r, g, b, a] = run.color;
                ctx.set_paint(AlphaColor::<Srgb>::from_rgba8(r, g, b, a));
                let glyphs = run.glyphs.iter().map(|&(id, x, y)| Glyph { id, x, y });
                // Ошибка отдельного глифа не роняет кадр: остальные глифы уже нарисованы.
                let _ = ctx
                    .glyph_run(&mut res, &run.font)
                    .font_size(run.size)
                    .normalized_coords(&run.coords)
                    .hint(false)
                    .fill_glyphs(glyphs);
            }
        }
    }

    let mut pixmap = Pixmap::new(w, h);
    ctx.flush();
    ctx.render(&mut pixmap, &mut res);
    Ok(pixmap)
}

pub fn execute_png(dl: &DisplayList, opts: &RenderOptions) -> Result<Vec<u8>> {
    Ok(execute(dl, opts)?.into_png()?)
}
