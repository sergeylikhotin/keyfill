//! Видео в файл: кадры диапазона рендерятся по порядку и уходят в ffmpeg через stdin.
//!
//! Флаги bitexact убирают из контейнера всё, что зависит от момента и версии кодировщика,
//! поэтому два прогона одной сцены дают одинаковые байты (проверяется тестом).

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
use kine_core::EvalOptions;
use kine_core::look::Rgba;
use kine_render::RenderOptions;

use crate::{Failure, Session};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    /// ProRes 4444 с альфой (.mov) — для эфира и монтажа.
    Prores,
    /// QuickTime Animation с альфой (.mov) — без потерь.
    Qtrle,
    /// VP9 с альфой (.webm) — смотреть в браузере.
    Vp9,
    /// H.264 без альфы (.mp4) — кадры кладутся на подложку.
    H264,
    /// PNG-последовательность в каталог, без ffmpeg.
    Png,
}

impl Codec {
    pub fn parse(s: &str) -> Result<Codec> {
        Ok(match s {
            "prores" => Codec::Prores,
            "qtrle" => Codec::Qtrle,
            "vp9" | "webm" => Codec::Vp9,
            "h264" | "mp4" => Codec::H264,
            "png" => Codec::Png,
            other => bail!("--codec: {other:?}; есть prores, qtrle, vp9, h264, png"),
        })
    }

    /// Кодек по расширению файла: .mov → prores, .webm → vp9, .mp4 → h264, без расширения → png.
    pub fn for_path(path: &Path) -> Result<Codec> {
        Ok(match path.extension().and_then(|e| e.to_str()) {
            Some("mov") => Codec::Prores,
            Some("webm") => Codec::Vp9,
            Some("mp4") => Codec::H264,
            None => Codec::Png,
            Some(other) => bail!("-o: расширение .{other} не поддерживается; .mov, .webm, .mp4 или каталог для PNG"),
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Codec::Prores => "prores 4444",
            Codec::Qtrle => "qtrle",
            Codec::Vp9 => "vp9",
            Codec::H264 => "h264",
            Codec::Png => "png",
        }
    }

    pub fn has_alpha(self) -> bool {
        self != Codec::H264
    }

    fn args(self) -> &'static [&'static str] {
        match self {
            Codec::Prores => &["-c:v", "prores_ks", "-profile:v", "4444", "-pix_fmt", "yuva444p10le", "-vendor", "apl0"],
            Codec::Qtrle => &["-c:v", "qtrle", "-pix_fmt", "argb"],
            Codec::Vp9 => &[
                "-c:v", "libvpx-vp9", "-pix_fmt", "yuva420p", "-b:v", "0", "-crf", "30", "-row-mt", "0", "-threads", "1", "-deadline", "good",
                "-cpu-used", "4", "-auto-alt-ref", "0",
            ],
            Codec::H264 => &["-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "18", "-preset", "medium", "-threads", "1"],
            Codec::Png => &[],
        }
    }
}

pub struct VideoOptions {
    pub from: f64,
    pub to: f64,
    pub codec: Codec,
    pub scale: f32,
    pub background: Option<Rgba>,
    pub out: PathBuf,
}

pub struct VideoReport {
    pub frames: usize,
    pub errors: usize,
}

/// Прямая альфа из premultiplied: кодировщики ждут RGBA без умножения.
fn unpremultiply(px: &mut [u8]) {
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

pub fn ffmpeg_available() -> bool {
    Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

pub fn render(session: &mut Session, eval: EvalOptions, opts: &VideoOptions) -> Result<VideoReport> {
    let fps = session.fps;
    let first = (opts.from * fps).round() as u64;
    let last = (opts.to * fps).round() as u64;
    if last < first {
        return Err(anyhow::anyhow!("--t: пустой диапазон").context(Failure::Usage));
    }
    // У H.264 нет альфы: без подложки кадры ложатся на чёрное.
    let background = match opts.codec {
        Codec::H264 => Some(opts.background.unwrap_or(Rgba { r: 0.0, g: 0.0, b: 0.0, a: 1.0 })),
        _ => opts.background,
    };
    let ropts = RenderOptions { scale: opts.scale, background };
    let mut errors = 0;

    if opts.codec == Codec::Png {
        std::fs::create_dir_all(&opts.out).with_context(|| format!("не удалось создать {}", opts.out.display()))?;
        for n in first..=last {
            let frame = session.frame_at(n as f64 / fps, eval)?;
            errors += frame.diagnostics.0.iter().filter(|d| d.level == kine_core::diag::Level::Error).count();
            let png = kine_render::render_png(&frame, &ropts)?;
            std::fs::write(opts.out.join(format!("{n:06}.png")), png)?;
        }
        return Ok(VideoReport { frames: (last - first + 1) as usize, errors });
    }

    if !ffmpeg_available() {
        return Err(anyhow::anyhow!("ffmpeg не найден в PATH; поставь ffmpeg или пиши PNG-последовательность: --codec png").context(Failure::Usage));
    }
    let w = (session.scene.stage.as_ref().map_or(1920.0, |s| s.size[0]) * opts.scale).round() as u32;
    let h = (session.scene.stage.as_ref().map_or(1080.0, |s| s.size[1]) * opts.scale).round() as u32;
    let mut child = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba"])
        .args(["-s", &format!("{w}x{h}"), "-r", &format!("{fps}"), "-i", "-", "-an"])
        .args(["-fflags", "+bitexact", "-flags:v", "+bitexact", "-map_metadata", "-1"])
        .args(opts.codec.args())
        .arg(&opts.out)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("не удалось запустить ffmpeg")?;
    let mut stdin = child.stdin.take().expect("stdin");
    for n in first..=last {
        let frame = session.frame_at(n as f64 / fps, eval)?;
        errors += frame.diagnostics.0.iter().filter(|d| d.level == kine_core::diag::Level::Error).count();
        let pixmap = kine_render::render(&frame, &ropts)?;
        let mut px = pixmap.data_as_u8_slice().to_vec();
        unpremultiply(&mut px);
        if stdin.write_all(&px).is_err() {
            break;
        }
    }
    drop(stdin);
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!("ffmpeg ({}): {}", opts.codec.name(), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(VideoReport { frames: (last - first + 1) as usize, errors })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn straight_alpha() {
        let mut px = [128, 64, 0, 128, 10, 20, 30, 0, 255, 255, 255, 255];
        unpremultiply(&mut px);
        assert_eq!(px, [255, 128, 0, 128, 0, 0, 0, 0, 255, 255, 255, 255]);
    }

    #[test]
    fn codec_by_extension() {
        assert_eq!(Codec::for_path(Path::new("a.mov")).unwrap(), Codec::Prores);
        assert_eq!(Codec::for_path(Path::new("a.webm")).unwrap(), Codec::Vp9);
        assert_eq!(Codec::for_path(Path::new("frames")).unwrap(), Codec::Png);
        assert!(Codec::for_path(Path::new("a.gif")).is_err());
    }
}
