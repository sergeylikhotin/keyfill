//! Конвейер сцены: TSX → esbuild → QuickJS → дерево → кадр во времени.

pub mod filmstrip;
pub mod session;
pub mod time;
pub mod video;
pub mod view;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result};
use kine_core::{EvalOptions, Frame, Scene};
use kine_js::{LogLine, RunOptions};

pub use session::Session;

/// Класс ошибки для кода выхода CLI: 2 — неверный вызов, 3 — сборка. Исключение JS
/// помечено типом `kine_js::JsException` (код 4), всё остальное — сбой kine (код 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    Usage,
    Build,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Failure::Usage => "неверный вызов",
            Failure::Build => "сборка сцены",
        })
    }
}

impl std::error::Error for Failure {}

/// Корень репозитория kine: там лежат `packages/core` и `node_modules/esbuild`.
pub fn kine_home() -> PathBuf {
    if let Some(home) = std::env::var_os("KINE_HOME") {
        return PathBuf::from(home);
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Собирает сцену в один IIFE-скрипт через `packages/core/bundle.mjs`: dev-JSX с местами
/// в исходнике, `@kine/core` из репозитория, импорты шрифтов как абсолютные пути к файлам.
/// Точка входа — сцена (`defineScene`) или сценарий (`cues`).
pub fn bundle(entry: &Path) -> Result<String> {
    let home = std::path::absolute(kine_home())?;
    if !home.join("node_modules/esbuild").exists() {
        return Err(anyhow::anyhow!("esbuild не найден в {}: выполни `npm install` в корне kine", slash(&home)).context(Failure::Build));
    }
    let entry = std::path::absolute(entry)?;
    if !entry.exists() {
        return Err(anyhow::anyhow!("сцена {} не найдена", slash(&entry)).context(Failure::Usage));
    }
    let out = Command::new("node")
        .arg(home.join("packages/core/bundle.mjs"))
        .arg(&entry)
        .stdin(Stdio::null())
        .output()
        .context("не удалось запустить node для сборки сцены")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim_end().to_string();
        return Err(anyhow::anyhow!("{stderr}").context(Failure::Build));
    }
    Ok(String::from_utf8(out.stdout)?)
}

pub struct Rendered {
    pub scene: Scene,
    pub frame: Frame,
    pub logs: Vec<LogLine>,
}

/// Кадр в момент `t`; без `t` — кадр «в эфире», когда закончились все `in` первого play.
pub fn frame_at(entry: &Path, opts: &RunOptions, eval: EvalOptions, t: Option<f64>) -> Result<Rendered> {
    let source = bundle(entry)?;
    let mut session = Session::open(&source, opts, &[])?;
    let t = match t {
        Some(t) => t,
        None => session.settled_time()?,
    };
    let frame = session.frame_at(t, eval)?;
    let Session { scene, logs, .. } = session;
    Ok(Rendered { scene, frame, logs })
}

/// Кадр «в эфире».
pub fn frame(entry: &Path, opts: &RunOptions, eval: EvalOptions) -> Result<Rendered> {
    frame_at(entry, opts, eval, None)
}
