//! Конвейер сцены: TSX → esbuild → QuickJS → дерево → кадр.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
use kine_core::{EvalOptions, Frame, Scene, TextEngine};
use kine_js::{LogLine, RunOptions};

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
pub fn bundle(entry: &Path) -> Result<String> {
    let home = std::path::absolute(kine_home())?;
    if !home.join("node_modules/esbuild").exists() {
        bail!("esbuild не найден в {}: выполни `npm install` в корне kine", slash(&home));
    }
    let entry = std::path::absolute(entry)?;
    if !entry.exists() {
        bail!("сцена {} не найдена", slash(&entry));
    }
    let out = Command::new("node")
        .arg(home.join("packages/core/bundle.mjs"))
        .arg(&entry)
        .stdin(Stdio::null())
        .output()
        .context("не удалось запустить node для сборки сцены")?;
    if !out.status.success() {
        bail!("сборка сцены:
{}", String::from_utf8_lossy(&out.stderr).trim_end());
    }
    Ok(String::from_utf8(out.stdout)?)
}

pub struct Loaded {
    pub scene: Scene,
    pub logs: Vec<LogLine>,
}

pub fn load(entry: &Path, opts: &RunOptions) -> Result<Loaded> {
    let source = bundle(entry)?;
    let output = kine_js::run_bundle(&source, opts)?;
    let mut scene = Scene::default();
    for batch in &output.batches {
        scene.apply_batch(batch)?;
    }
    Ok(Loaded { scene, logs: output.logs })
}

pub struct Rendered {
    pub scene: Scene,
    pub frame: Frame,
    pub logs: Vec<LogLine>,
}

/// Полный кадр: шрифты сцены, раскладка, `must` и `else`.
pub fn frame(entry: &Path, opts: &RunOptions, eval: EvalOptions) -> Result<Rendered> {
    let Loaded { scene, logs } = load(entry, opts)?;
    let mut engine = TextEngine::for_scene(&scene);
    let frame = kine_core::evaluate(&scene, &mut engine, eval)?;
    Ok(Rendered { scene, frame, logs })
}
