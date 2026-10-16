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
pub use vello_cpu::Pixmap;
use vello_cpu::{Glyph, RenderContext, Resources};

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

/// Рендер кадра одним вызовом: строит display list и исполняет его.
pub fn render(frame: &Frame, opts: &RenderOptions) -> Result<Pixmap> {
    execute(&DisplayList::from_frame(frame), opts)
}

/// PNG кадра с компактным сжатием (для файлов, которые остаются).
pub fn render_png(frame: &Frame, opts: &RenderOptions) -> Result<Vec<u8>> {
    png(&render(frame, opts)?, Png::Small)
}

pub fn execute(dl: &DisplayList, opts: &RenderOptions) -> Result<Pixmap> {
    let mut r = Renderer::default();
    r.execute(dl, opts)?;
    Ok(std::mem::replace(&mut r.pixmap, Pixmap::new(1, 1)))
}

pub fn execute_png(dl: &DisplayList, opts: &RenderOptions) -> Result<Vec<u8>> {
    png(&execute(dl, opts)?, Png::Small)
}

/// Рендерер с состоянием: контекст, кэш глифов и буфер кадра живут между кадрами (видео,
/// просмотрщик) и не создаются заново на каждый кадр. Пиксели те же, что у `execute`.
pub struct Renderer {
    ctx: Option<RenderContext>,
    res: Resources,
    pixmap: Pixmap,
}

impl Default for Renderer {
    fn default() -> Self {
        Renderer { ctx: None, res: Resources::new(), pixmap: Pixmap::new(1, 1) }
    }
}

impl Renderer {
    pub fn render(&mut self, frame: &Frame, opts: &RenderOptions) -> Result<&Pixmap> {
        self.execute(&DisplayList::from_frame(frame), opts)
    }

    pub fn execute(&mut self, dl: &DisplayList, opts: &RenderOptions) -> Result<&Pixmap> {
        let w = (dl.size[0] * opts.scale).round();
        let h = (dl.size[1] * opts.scale).round();
        if !(1.0..=u16::MAX as f32).contains(&w) || !(1.0..=u16::MAX as f32).contains(&h) {
            bail!("размер выхода {w}×{h} вне допустимого диапазона");
        }
        let (w, h) = (w as u16, h as u16);
        let ctx = match &mut self.ctx {
            Some(ctx) if ctx.width() == w && ctx.height() == h => {
                ctx.reset();
                ctx
            }
            slot => slot.insert(RenderContext::new(w, h)),
        };
        draw(ctx, &mut self.res, dl, opts, w, h);
        if self.pixmap.width() != w || self.pixmap.height() != h {
            self.pixmap = Pixmap::new(w, h);
        } else {
            // Рендер смешивает поверх буфера: прошлый кадр нужно стереть.
            self.pixmap.data_as_u8_slice_mut().fill(0);
        }
        ctx.flush();
        ctx.render(&mut self.pixmap, &mut self.res);
        Ok(&self.pixmap)
    }
}

/// Сжатие PNG: компактное для файлов, которые остаются, быстрое — для просмотрщика и кадров видео.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Png {
    Small,
    Fast,
}

/// PNG из буфера кадра: прямая альфа; без прозрачности — RGB.
pub fn png(pixmap: &Pixmap, speed: Png) -> Result<Vec<u8>> {
    let mut px = pixmap.data_as_u8_slice().to_vec();
    let opaque = px.chunks_exact(4).all(|p| p[3] == 255);
    let (color, data) = if opaque {
        (png::ColorType::Rgb, px.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect())
    } else {
        unpremultiply(&mut px);
        (png::ColorType::Rgba, px)
    };
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, u32::from(pixmap.width()), u32::from(pixmap.height()));
    encoder.set_color(color);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(match speed {
        Png::Small => png::Compression::Balanced,
        Png::Fast => png::Compression::Fast,
    });
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&data)?;
    writer.finish()?;
    Ok(out)
}

/// Прямая альфа из premultiplied: PNG и кодировщики видео ждут RGBA без умножения.
pub fn unpremultiply(px: &mut [u8]) {
    for p in px.chunks_exact_mut(4) {
        let a = p[3] as u32;
        if a == 0 {
            p[..3].fill(0);
        } else if a < 255 {
            for c in &mut p[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
}

fn draw(ctx: &mut RenderContext, res: &mut Resources, dl: &DisplayList, opts: &RenderOptions, w: u16, h: u16) {
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
                    .glyph_run(res, &run.font)
                    .font_size(run.size)
                    .normalized_coords(&run.coords)
                    .hint(false)
                    .fill_glyphs(glyphs);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(x: f32, c: Rgba) -> DisplayList {
        let mut dl = DisplayList::new([64.0, 32.0]);
        dl.items.push(Item::Fill { transform: Affine::IDENTITY, rect: Rect { x, y: 4.0, w: 20.0, h: 20.0 }, radii: [4.0; 4], color: c });
        dl
    }

    #[test]
    fn reused_renderer_matches_fresh_render() {
        let a = list(4.0, Rgba { r: 1.0, g: 0.0, b: 0.0, a: 0.5 });
        let b = list(30.0, Rgba::WHITE);
        let opts = RenderOptions::default();
        let mut r = Renderer::default();
        r.execute(&a, &opts).unwrap();
        let reused = r.execute(&b, &opts).unwrap().data_as_u8_slice().to_vec();
        assert_eq!(reused, execute(&b, &opts).unwrap().data_as_u8_slice(), "буфер прошлого кадра протёк в новый");
    }

    #[test]
    fn straight_alpha() {
        let mut px = [128, 64, 0, 128, 10, 20, 30, 0, 255, 255, 255, 255];
        unpremultiply(&mut px);
        assert_eq!(px, [255, 128, 0, 128, 0, 0, 0, 0, 255, 255, 255, 255]);
    }
}
