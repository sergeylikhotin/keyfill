//! Конвейер сцены: TSX → esbuild → QuickJS → дерево → кадр во времени.

pub mod bundle;
pub mod filmstrip;
pub mod ops;
pub mod output;
pub mod session;
pub mod sourcemap;
pub mod time;
pub mod video;
pub mod view;

use std::path::{Path, PathBuf};

use anyhow::Result;
use kine_core::{EvalOptions, Frame, Scene};
use kine_js::{LogLine, RunOptions};

pub use bundle::{Bundle, bundle};
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

/// Помечает ошибку как неверный вызов, если она ещё не классифицирована: исключение JS
/// и сборка сохраняют свой класс (иначе в сообщении появился бы лишний префикс).
pub fn usage(e: anyhow::Error) -> anyhow::Error {
    if e.downcast_ref::<kine_js::JsException>().is_some() || e.downcast_ref::<Failure>().is_some() {
        e
    } else {
        e.context(Failure::Usage)
    }
}

/// Корень репозитория kine: там лежат `packages/core` и `node_modules/esbuild`.
pub fn kine_home() -> PathBuf {
    if let Some(home) = std::env::var_os("KINE_HOME") {
        return PathBuf::from(home);
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

pub(crate) fn slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
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
