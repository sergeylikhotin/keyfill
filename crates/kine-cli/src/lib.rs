//! Конвейер сцены: TSX → esbuild → QuickJS → дерево → кадр во времени.

pub mod bundle;
pub mod config;
pub mod filmstrip;
pub mod help;
pub mod ops;
pub mod output;
pub mod session;
pub mod sourcemap;
pub mod suite;
pub mod time;
pub mod video;
pub mod view;

use std::path::{Path, PathBuf};

pub use bundle::{Bundle, bundle};
pub use session::Session;

/// Ошибка с известным классом: его задаёт место, где ошибка возникла, а код выхода и
/// `error.kind` берутся из него. Исключение JS — `kine_js::JsException` (код 4), всё
/// остальное — сбой kine (код 5).
#[derive(Debug)]
pub enum Failure {
    /// Неверный вызов: флаги, время, адрес узла, сценарий (код 2).
    Usage(String),
    /// Данные сцены не подходят к её схеме (код 2, подсказка `kine schema`).
    Props(Vec<String>),
    /// Сборка сцены: диагностики `build.*` с местом в исходнике (код 3).
    Build(Vec<kine_core::diag::Diagnostic>),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Usage(m) => f.write_str(m),
            Failure::Props(errors) => write!(f, "пропсы сцены:\n  {}", errors.join("\n  ")),
            Failure::Build(diags) => {
                let lines: Vec<String> = diags.iter().map(|d| d.to_string()).collect();
                f.write_str(&lines.join("\n"))
            }
        }
    }
}

impl std::error::Error for Failure {}

/// Ошибка неверного вызова.
pub fn usage(message: impl std::fmt::Display) -> anyhow::Error {
    Failure::Usage(message.to_string()).into()
}

/// Корень репозитория kine: там лежат `packages/core` и `node_modules/esbuild`.
pub fn kine_home() -> PathBuf {
    if let Some(home) = std::env::var_os("KINE_HOME") {
        return PathBuf::from(home);
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// Путь с прямыми слешами: одинаково на Windows и в выводе для агента.
pub fn slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}
