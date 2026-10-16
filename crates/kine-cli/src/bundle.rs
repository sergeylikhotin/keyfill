//! Сборка сцены через `packages/core/bundle.mjs` и кэш бандла.
//!
//! Кэш лежит в `.kine/cache` каталога запуска (или в `KINE_CACHE_DIR`). Запись годна, пока
//! не изменилось содержимое ни одного входного файла: сцены, компонентов, шрифтов, `@kine/core`
//! и самого бандлера. Тогда node не запускается вовсе.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result};
use kine_core::diag::{Diagnostic, Level};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::sourcemap::SourceMap;
use crate::{Failure, kine_home, slash};

/// Собранная сцена.
#[derive(Clone)]
pub struct Bundle {
    pub code: String,
    pub map: Option<SourceMap>,
    /// Все входные файлы сборки: по ним кэш и просмотрщик узнают о правках.
    pub inputs: Vec<PathBuf>,
}

#[derive(Deserialize)]
struct Output {
    code: String,
    map: Value,
    inputs: Vec<String>,
}

#[derive(Deserialize)]
struct Errors {
    errors: Vec<EsbuildError>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EsbuildError {
    file: Option<String>,
    line: Option<u32>,
    column: Option<u32>,
    text: String,
    line_text: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    stamp: String,
    inputs: Vec<(String, String)>,
    code: String,
    map: Value,
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn hash_file(path: &Path) -> Option<String> {
    std::fs::read(path).ok().map(|b| hash_bytes(&b))
}

fn cache_dir() -> Option<PathBuf> {
    if std::env::var_os("KINE_NO_CACHE").is_some() {
        return None;
    }
    match std::env::var_os("KINE_CACHE_DIR") {
        Some(dir) => Some(PathBuf::from(dir)),
        None => std::env::current_dir().ok().map(|d| d.join(".kine").join("cache")),
    }
}

/// Версия бандлера: kine и сам `bundle.mjs`. Смена любого из них сбрасывает кэш.
fn stamp(script: &Path) -> String {
    format!("{}+{}", env!("CARGO_PKG_VERSION"), hash_file(script).unwrap_or_default())
}

fn cache_file(dir: &Path, entry: &Path) -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_default();
    let key = hash_bytes(format!("{}|{}", slash(entry), slash(&cwd)).as_bytes());
    dir.join(format!("{key}.json"))
}

fn read_cache(file: &Path, stamp: &str) -> Option<Bundle> {
    let entry: CacheEntry = serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
    if entry.stamp != stamp {
        return None;
    }
    for (path, hash) in &entry.inputs {
        if hash_file(Path::new(path)).as_ref() != Some(hash) {
            return None;
        }
    }
    Some(Bundle {
        map: SourceMap::parse(&entry.map),
        code: entry.code,
        inputs: entry.inputs.into_iter().map(|(p, _)| PathBuf::from(p)).collect(),
    })
}

fn write_cache(file: &Path, stamp: String, out: &Output) {
    let inputs = out.inputs.iter().filter_map(|p| Some((p.clone(), hash_file(Path::new(p))?))).collect();
    let entry = CacheEntry { stamp, inputs, code: out.code.clone(), map: out.map.clone() };
    let Ok(json) = serde_json::to_vec(&entry) else { return };
    // Через временный файл: параллельный запуск не прочтёт половину записи.
    let tmp = file.with_extension(format!("{}.tmp", std::process::id()));
    if std::fs::create_dir_all(file.parent().unwrap_or(Path::new("."))).is_ok() && std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(&tmp, file);
    }
}

/// Сбой сборки без места в исходнике: нет node или esbuild, бандлер упал.
fn tool_error(rule: &str, message: String) -> anyhow::Error {
    let d = Diagnostic { level: Level::Error, rule: rule.into(), node: "build".into(), source: None, message, hint: None, detail: None };
    Failure::Build(vec![d]).into()
}

fn build_diagnostic(e: EsbuildError) -> Diagnostic {
    let (rule, hint) = if e.text.starts_with("Could not resolve") {
        ("build.resolve", Some("проверь путь импорта; пакеты из npm ставятся через npm install".to_string()))
    } else {
        ("build.syntax", None)
    };
    let source = match (&e.file, e.line, e.column) {
        (Some(f), Some(l), Some(c)) => Some(format!("{}:{l}:{c}", slash(Path::new(f)))),
        _ => None,
    };
    Diagnostic {
        level: Level::Error,
        rule: rule.into(),
        node: "build".into(),
        detail: e.line_text.as_ref().map(|t| serde_json::json!({ "line": t })),
        source,
        message: e.text,
        hint,
    }
}

/// Собирает сцену в один IIFE-скрипт через `packages/core/bundle.mjs`: dev-JSX с местами
/// в исходнике, `@kine/core` из репозитория, импорты шрифтов как абсолютные пути к файлам.
/// Точка входа — сцена (`defineScene`) или сценарий (`cues`).
pub fn bundle(entry: &Path) -> Result<Bundle> {
    let home = std::path::absolute(kine_home())?;
    let entry = std::path::absolute(entry)?;
    if !entry.exists() {
        return Err(crate::usage(format!("сцена {} не найдена", slash(&entry))));
    }
    let script = home.join("packages/core/bundle.mjs");
    let stamp = stamp(&script);
    let cache = cache_dir().map(|d| cache_file(&d, &entry));
    if let Some(hit) = cache.as_deref().and_then(|f| read_cache(f, &stamp)) {
        return Ok(hit);
    }
    if !home.join("node_modules/esbuild").exists() {
        let msg = format!("esbuild не найден в {}: выполни `npm install` в корне kine", slash(&home));
        return Err(tool_error("build.setup", msg));
    }
    let out = match Command::new("node").arg(&script).arg(&entry).stdin(Stdio::null()).output() {
        Ok(out) => out,
        Err(e) => return Err(tool_error("build.setup", format!("не удалось запустить node для сборки сцены: {e}"))),
    };
    if !out.status.success() {
        if let Ok(errs) = serde_json::from_slice::<Errors>(&out.stdout) {
            return Err(Failure::Build(errs.errors.into_iter().map(build_diagnostic).collect()).into());
        }
        return Err(tool_error("build.error", String::from_utf8_lossy(&out.stderr).trim_end().to_string()));
    }
    let parsed: Output = serde_json::from_slice(&out.stdout).context("бандлер вернул не JSON")?;
    if let Some(file) = &cache {
        write_cache(file, stamp, &parsed);
    }
    Ok(Bundle {
        map: SourceMap::parse(&parsed.map),
        inputs: parsed.inputs.iter().map(PathBuf::from).collect(),
        code: parsed.code,
    })
}
