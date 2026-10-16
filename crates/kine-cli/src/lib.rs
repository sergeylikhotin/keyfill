//! Конвейер сцены: TSX → esbuild → QuickJS → дерево → кадр во времени.

pub mod bundle;
pub mod config;
pub mod doctor;
pub mod filmstrip;
pub mod help;
pub mod ops;
pub mod output;
pub mod scaffold;
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

/// Корень репозитория kine (`KINE_HOME` или путь сборки): там лежат `packages/core`
/// и `node_modules/esbuild`. Нужен разработке и тестам; установленный kine берёт `@kine/core`
/// через `core_dir`.
pub fn kine_home() -> PathBuf {
    if let Some(home) = std::env::var_os("KINE_HOME") {
        return PathBuf::from(home);
    }
    // Без `..` в пути: он попадает в вывод `kine doctor`.
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().and_then(Path::parent).unwrap_or(manifest).to_path_buf()
}

/// Путь для вывода: от каталога запуска, если он внутри, с прямыми слешами.
pub fn shown(p: &Path) -> String {
    let cwd = std::env::current_dir().unwrap_or_default();
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    match abs.strip_prefix(&cwd) {
        Ok(rel) if rel.as_os_str().is_empty() => ".".into(),
        Ok(rel) => slash(rel),
        Err(_) => slash(&abs),
    }
}

/// Откуда взят `@kine/core`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CoreSource {
    /// `KINE_CORE`: его ставит `npx kine` (пакет `@kine/cli`).
    Env,
    /// `node_modules/@kine/core` вверх от каталога запуска.
    NodeModules,
    /// `packages/core` репозитория kine.
    Repo,
}

/// Каталог `@kine/core`: бандлер `bundle.mjs` и исходники `src`. По порядку: `KINE_CORE`,
/// `node_modules/@kine/core` вверх от каталога запуска, `packages/core` репозитория.
pub fn core_dir() -> (PathBuf, CoreSource) {
    if let Some(dir) = std::env::var_os("KINE_CORE") {
        return (PathBuf::from(dir), CoreSource::Env);
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    if let Some(dir) = cwd.ancestors().map(|d| d.join("node_modules/@kine/core")).find(|d| d.join("bundle.mjs").is_file()) {
        return (dir, CoreSource::NodeModules);
    }
    (kine_home().join("packages/core"), CoreSource::Repo)
}

/// `esbuild`, как его найдёт node из `bundle.mjs`: `node_modules/esbuild` вверх от `@kine/core`.
pub fn esbuild_dir(core: &Path) -> Option<PathBuf> {
    core.ancestors().map(|d| d.join("node_modules/esbuild")).find(|d| d.join("package.json").is_file())
}

/// Путь с прямыми слешами: одинаково на Windows и в выводе для агента.
pub fn slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}
