//! Конвейер сцены: TSX → esbuild → QuickJS → дерево → кадр.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
use kine_core::{Frame, Scene, TextEngine};
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

/// Собирает сцену в один IIFE-скрипт. JSX компилируется в dev-режиме, чтобы у каждого
/// элемента было место в исходнике.
pub fn bundle(entry: &Path) -> Result<String> {
    let home = std::path::absolute(kine_home())?;
    let esbuild = home.join("node_modules/esbuild/bin/esbuild");
    if !esbuild.exists() {
        bail!("esbuild не найден в {}: выполни `npm install` в корне kine", slash(&home));
    }
    let entry = std::path::absolute(entry)?;
    if !entry.exists() {
        bail!("сцена {} не найдена", slash(&entry));
    }
    let core = home.join("packages/core/src");
    let stdin = format!("import scene from {:?};\nimport {{ mount }} from \"@kine/core\";\nmount(scene);\n", slash(&entry));

    let mut child = Command::new("node")
        .arg(&esbuild)
        .args([
            "--bundle",
            "--format=iife",
            "--platform=neutral",
            "--target=es2020",
            "--jsx=automatic",
            "--jsx-dev",
            "--jsx-import-source=@kine/core",
            "--loader=ts",
            "--sourcefile=kine-entry.ts",
            "--log-level=warning",
        ])
        .arg(format!("--alias:@kine/core={}", slash(&core)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("не удалось запустить node для esbuild")?;
    child.stdin.take().expect("stdin piped").write_all(stdin.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!("esbuild:\n{}", String::from_utf8_lossy(&out.stderr));
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

pub fn frame(entry: &Path, opts: &RunOptions) -> Result<Rendered> {
    let Loaded { scene, logs } = load(entry, opts)?;
    let mut engine = TextEngine::default();
    let frame = kine_core::build_frame(&scene, &mut engine)?;
    Ok(Rendered { scene, frame, logs })
}
