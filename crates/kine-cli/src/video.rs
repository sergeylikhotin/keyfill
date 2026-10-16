//! Видео в файл: кадры диапазона рендерятся по порядку и уходят в ffmpeg через stdin.
//!
//! Флаги bitexact убирают из контейнера всё, что зависит от момента и версии кодировщика,
//! поэтому два прогона одной сцены дают одинаковые байты (проверяется тестом).

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
use kine_core::EvalOptions;
use kine_core::diag::Counts;
use kine_core::look::Rgba;
use kine_render::{Png, RenderOptions, Renderer};

use crate::Session;

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

    /// Число потоков задано явно, а не по ядрам машины: при фиксированных потоках и тайлах
    /// кодировщики дают одинаковые байты на любой машине (VP9 с row-mt — при любом числе потоков).
    fn args(self) -> &'static [&'static str] {
        match self {
            Codec::Prores => &["-c:v", "prores_ks", "-profile:v", "4444", "-pix_fmt", "yuva444p10le", "-vendor", "apl0"],
            Codec::Qtrle => &["-c:v", "qtrle", "-pix_fmt", "argb"],
            Codec::Vp9 => &[
                "-c:v", "libvpx-vp9", "-pix_fmt", "yuva420p", "-b:v", "0", "-crf", "30", "-row-mt", "1", "-tile-columns", "2", "-threads", "8", "-deadline", "good",
                "-cpu-used", "4", "-auto-alt-ref", "0",
            ],
            Codec::H264 => &["-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "18", "-preset", "medium", "-threads", "8"],
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
    /// Кадров с ошибками диагностики.
    pub errors: usize,
}

pub fn ffmpeg_available() -> bool {
    Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// Куда уходят кадры: PNG-файлы в каталог или ffmpeg через stdin.
enum Sink {
    Dir(PathBuf),
    Ffmpeg { child: std::process::Child, stdin: std::process::ChildStdin, buf: Vec<u8> },
}

impl Sink {
    fn open(session: &Session, opts: &VideoOptions) -> Result<Sink> {
        if opts.codec == Codec::Png {
            std::fs::create_dir_all(&opts.out).map_err(|e| crate::usage(format!("не удалось создать {}: {e}", opts.out.display())))?;
            return Ok(Sink::Dir(opts.out.clone()));
        }
        if !ffmpeg_available() {
            return Err(crate::usage("ffmpeg не найден в PATH; поставь ffmpeg или пиши PNG-последовательность: --codec png"));
        }
        let size = session.scene.stage.as_ref().map_or([1920.0, 1080.0], |s| s.size);
        let [w, h] = size.map(|v| (v * opts.scale).round() as u32);
        let mut child = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba"])
            .args(["-s", &format!("{w}x{h}"), "-r", &format!("{}", session.fps), "-i", "-", "-an"])
            .args(["-fflags", "+bitexact", "-flags:v", "+bitexact", "-map_metadata", "-1"])
            .args(opts.codec.args())
            .arg(&opts.out)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("не удалось запустить ffmpeg")?;
        let stdin = child.stdin.take().expect("stdin");
        Ok(Sink::Ffmpeg { child, stdin, buf: Vec::new() })
    }

    /// Записывает кадр; `false` — ffmpeg закрыл вход (ошибку он сообщит в `finish`).
    fn write(&mut self, n: u64, pixmap: &kine_render::Pixmap) -> Result<bool> {
        match self {
            Sink::Dir(dir) => {
                std::fs::write(dir.join(format!("{n:06}.png")), kine_render::png(pixmap, Png::Fast)?)?;
                Ok(true)
            }
            Sink::Ffmpeg { stdin, buf, .. } => {
                buf.clear();
                buf.extend_from_slice(pixmap.data_as_u8_slice());
                kine_render::unpremultiply(buf);
                Ok(stdin.write_all(buf).is_ok())
            }
        }
    }

    fn finish(self, codec: Codec) -> Result<()> {
        if let Sink::Ffmpeg { child, stdin, .. } = self {
            drop(stdin);
            let out = child.wait_with_output()?;
            if !out.status.success() {
                bail!("ffmpeg ({}): {}", codec.name(), String::from_utf8_lossy(&out.stderr).trim());
            }
        }
        Ok(())
    }
}

pub fn render(session: &mut Session, eval: EvalOptions, opts: &VideoOptions) -> Result<VideoReport> {
    let frames = session.frames(opts.from, opts.to);
    if frames.is_empty() {
        return Err(crate::usage("--t: пустой диапазон"));
    }
    // У H.264 нет альфы: без подложки кадры ложатся на чёрное.
    let background = match opts.codec {
        Codec::H264 => Some(opts.background.unwrap_or(Rgba { r: 0.0, g: 0.0, b: 0.0, a: 1.0 })),
        _ => opts.background,
    };
    let ropts = RenderOptions { scale: opts.scale, background };
    let mut sink = Sink::open(session, opts)?;
    let mut renderer = Renderer::default();
    let mut errors = 0;
    for n in frames.clone() {
        let frame = session.frame_at(session.frame_time(n), eval)?;
        errors += usize::from(Counts::of(&frame.diagnostics.0).error > 0);
        if !sink.write(n, renderer.render(&frame, &ropts)?)? {
            break;
        }
    }
    sink.finish(opts.codec)?;
    Ok(VideoReport { frames: frames.count(), errors })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_by_extension() {
        assert_eq!(Codec::for_path(Path::new("a.mov")).unwrap(), Codec::Prores);
        assert_eq!(Codec::for_path(Path::new("a.webm")).unwrap(), Codec::Vp9);
        assert_eq!(Codec::for_path(Path::new("frames")).unwrap(), Codec::Png);
        assert!(Codec::for_path(Path::new("a.gif")).is_err());
    }
}
