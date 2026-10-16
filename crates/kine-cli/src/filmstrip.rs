//! Раскадровка: несколько моментов сцены одной картинкой. Собирается на уровне display
//! list: каждый кадр вставляется в общий список со своим масштабом и сдвигом.

use anyhow::Result;
use kine_core::display::{Affine, DisplayList, Item};
use kine_core::look::Rgba;
use kine_core::{EvalOptions, Frame, Rect};

use crate::Session;

pub struct Options {
    /// Диапазон в секундах (уже разрешённый из `--t a..b`).
    pub from: f64,
    pub to: f64,
    pub count: usize,
    pub cols: usize,
    /// Ширина плитки в пикселях.
    pub tile: f32,
    /// Подложка кадров; без неё — шахматка, чтобы была видна прозрачность.
    pub background: Option<Rgba>,
}

const MARGIN: f32 = 12.0;
const GAP: f32 = 12.0;
const LABEL: f32 = 26.0;
const CELL: f32 = 12.0;

fn rgb(r: u8, g: u8, b: u8) -> Rgba {
    Rgba::from_u8(r, g, b)
}

/// Моменты раскадровки: равномерно от `from` до `to` включительно, на границах кадров.
pub fn times(session: &Session, opts: &Options) -> Vec<f64> {
    let n = opts.count.max(1);
    let mut out: Vec<f64> = (0..n)
        .map(|i| {
            let t = if n == 1 { opts.from } else { opts.from + (opts.to - opts.from) * i as f64 / (n - 1) as f64 };
            // Ближайший кадр, но не позже `to`.
            session.nearest_frame(t).min(opts.to.max(opts.from))
        })
        .collect();
    out.dedup();
    out
}

pub struct Strip {
    pub display: DisplayList,
    pub frames: Vec<Frame>,
    /// Дамп каждого кадра, снятый в его момент (данные сцены могли меняться между кадрами).
    pub dumps: Vec<kine_core::dump::Dump>,
}

/// Кадры в моменты `times` и их общая раскадровка.
pub fn build(session: &mut Session, times: &[f64], eval: EvalOptions, opts: &Options) -> Result<Strip> {
    let cols = opts.cols.clamp(1, times.len().max(1));
    let rows = times.len().div_ceil(cols);
    let mut frames = Vec::with_capacity(times.len());
    let mut dumps = Vec::with_capacity(times.len());
    for &t in times {
        let frame = session.frame_at(t, eval)?;
        dumps.push(kine_core::dump::build(&session.scene, &frame));
        frames.push(frame);
    }
    let size = frames.first().map_or([1920.0, 1080.0], |f| f.size);
    let scale = opts.tile / size[0];
    let tile = [opts.tile, (size[1] * scale).round()];
    let width = 2.0 * MARGIN + cols as f32 * tile[0] + (cols - 1) as f32 * GAP;
    let height = 2.0 * MARGIN + rows as f32 * (LABEL + tile[1]) + (rows - 1) as f32 * GAP;

    let mut dl = DisplayList::new([width, height]);
    let id = Affine::IDENTITY;
    dl.items.push(Item::Fill { transform: id, rect: Rect { x: 0.0, y: 0.0, w: width, h: height }, radii: [0.0; 4], color: rgb(22, 24, 29) });

    for (i, frame) in frames.iter().enumerate() {
        let (col, row) = (i % cols, i / cols);
        let x = MARGIN + col as f32 * (tile[0] + GAP);
        let y = MARGIN + row as f32 * (LABEL + tile[1] + GAP);
        let area = Rect { x, y: y + LABEL, w: tile[0], h: tile[1] };

        let label = match &frame.clock {
            Some(c) => format!("t={:.2}s  кадр {}  {}", c.t, c.frame, c.scene),
            None => format!("кадр {i}"),
        };
        if let Some(block) = session.label(&label, 15.0, rgb(208, 212, 220)) {
            dl.text(&block, [x, y + 3.0], id, 1.0);
        }

        dl.items.push(Item::PushClip { transform: id, rect: area });
        match opts.background {
            Some(bg) => dl.items.push(Item::Fill { transform: id, rect: area, radii: [0.0; 4], color: bg }),
            None => checker(&mut dl, area),
        }
        let frame_dl = DisplayList::from_frame(frame);
        dl.append(&frame_dl, Affine::translate(x as f64, (y + LABEL) as f64) * Affine::scale(scale as f64, scale as f64));
        dl.items.push(Item::Pop);
    }
    Ok(Strip { display: dl, frames, dumps })
}

fn checker(dl: &mut DisplayList, area: Rect) {
    let id = Affine::IDENTITY;
    dl.items.push(Item::Fill { transform: id, rect: area, radii: [0.0; 4], color: rgb(58, 60, 66) });
    let (nx, ny) = ((area.w / CELL).ceil() as usize, (area.h / CELL).ceil() as usize);
    for j in 0..ny {
        for i in (j % 2..nx).step_by(2) {
            let rect = Rect { x: area.x + i as f32 * CELL, y: area.y + j as f32 * CELL, w: CELL, h: CELL };
            dl.items.push(Item::Fill { transform: id, rect, radii: [0.0; 4], color: rgb(44, 46, 52) });
        }
    }
}
