//! `keyfill.config.ts`: значения по умолчанию для команд. Ищется вверх от каталога запуска;
//! собирается и исполняется как сцена (бандл кэшируется), `defineConfig` отдаёт значения
//! ядру операцией `config`. Флаги перекрывают конфиг.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use keyfill_js::{JsHost, RunOptions};
use serde::Deserialize;

use crate::{Failure, slash};

pub const FILE: &str = "keyfill.config.ts";

/// Значения конфига; пути уже от каталога конфига.
#[derive(Debug, Default, Clone)]
pub struct Config {
    /// Файл конфига; `None` — конфига нет.
    pub file: Option<PathBuf>,
    pub mode: Option<String>,
    pub bg: Option<String>,
    pub seed: Option<u32>,
    pub now: Option<f64>,
    pub out: Option<PathBuf>,
    pub scenes: BTreeMap<String, PathBuf>,
    /// Каналы `keyfill serve`: номер → файл канала (`defineChannel`).
    pub channels: BTreeMap<u32, PathBuf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    mode: Option<String>,
    bg: Option<String>,
    seed: Option<u32>,
    now: Option<f64>,
    out: Option<String>,
    #[serde(default)]
    scenes: BTreeMap<String, String>,
    #[serde(default)]
    channels: BTreeMap<String, String>,
}

/// Ближайший `keyfill.config.ts` вверх от `dir`.
pub fn find(dir: &Path) -> Option<PathBuf> {
    dir.ancestors().map(|d| d.join(FILE)).find(|f| f.is_file())
}

/// Конфиг для каталога запуска; без файла — пустой.
pub fn load() -> Result<Config> {
    let cwd = std::env::current_dir()?;
    match find(&cwd) {
        Some(file) => read(&file),
        None => Ok(Config::default()),
    }
}

pub fn read(file: &Path) -> Result<Config> {
    let bundle = crate::bundle(file)?;
    let (_, out) = JsHost::start(&bundle.code, &RunOptions::default()).map_err(|e| crate::session::map_js(bundle.map.as_ref(), e))?;
    let shown = shown(file);
    let value = out
        .batches
        .iter()
        .filter_map(|b| serde_json::from_str::<Vec<serde_json::Value>>(b).ok())
        .flatten()
        .find(|c| c["op"] == "config")
        .map(|c| c["value"].clone())
        .ok_or_else(|| crate::usage(format!("{shown}: ожидается export default defineConfig({{ ... }})")))?;
    let raw: Raw = serde_json::from_value(value).map_err(|e| crate::usage(format!("{shown}: {e}")))?;
    let dir = file.parent().unwrap_or(Path::new("."));
    if let Some(m) = raw.mode.as_deref().filter(|m| !["dev", "air"].contains(m)) {
        return Err(crate::usage(format!("{shown}: mode — dev или air, получено {m}")));
    }
    let mut channels = BTreeMap::new();
    for (k, v) in raw.channels {
        let n: u32 = k.parse().ok().filter(|n| *n > 0).ok_or_else(|| crate::usage(format!("{shown}: channels — номера каналов от 1, получено {k:?}")))?;
        channels.insert(n, dir.join(v));
    }
    Ok(Config {
        file: Some(file.to_path_buf()),
        channels,
        mode: raw.mode,
        bg: raw.bg,
        seed: raw.seed,
        now: raw.now,
        out: raw.out.map(|o| dir.join(o)),
        scenes: raw.scenes.into_iter().map(|(k, v)| (k, dir.join(v))).collect(),
    })
}

impl Config {
    /// Файл сцены: путь к файлу, имя сцены из конфига (`keyfill render lower`) или каталог
    /// со `scene.cues.ts` или `scene.tsx` (так их создаёт `keyfill new`).
    pub fn entry(&self, arg: &str) -> Result<PathBuf> {
        let path = PathBuf::from(arg);
        if path.is_file() {
            return Ok(path);
        }
        if let Some(p) = self.scenes.get(arg) {
            return Ok(relative(p));
        }
        if let Some(file) = ["scene.cues.ts", "scene.tsx", "channel.tsx"].iter().map(|f| path.join(f)).find(|f| f.is_file()) {
            return Ok(file);
        }
        let named = if self.scenes.is_empty() {
            String::new()
        } else {
            format!("; именованные сцены: {}", self.scenes.keys().cloned().collect::<Vec<_>>().join(", "))
        };
        Err(Failure::Usage(format!("сцена {arg} не найдена{named}")).into())
    }

    /// Файл результата без `-o`: в каталоге `out` конфига с путём сцены внутри
    /// (`out/examples/lower-third/scene.png`), без конфига — рядом со сценой.
    pub fn output(&self, entry: &Path, ext: &str) -> Result<PathBuf> {
        let Some(out) = &self.out else { return Ok(entry.with_extension(ext)) };
        let base = self.file.as_deref().and_then(Path::parent).unwrap_or(Path::new("."));
        let abs = std::path::absolute(entry)?;
        let rel = abs.strip_prefix(std::path::absolute(base)?).map(Path::to_path_buf).unwrap_or_else(|_| PathBuf::from(abs.file_name().unwrap_or_default()));
        let file = relative(&out.join(rel)).with_extension(ext);
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("не удалось создать {}", dir.display()))?;
        }
        Ok(file)
    }

    /// Путь конфига для вывода.
    pub fn shown(&self) -> Option<String> {
        self.file.as_deref().map(shown)
    }
}

/// Путь от каталога запуска, если файл внутри него: так короче в выводе и в `repro`.
fn relative(p: &Path) -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_default();
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    abs.strip_prefix(&cwd).map(Path::to_path_buf).unwrap_or(abs)
}

fn shown(p: &Path) -> String {
    slash(&relative(p))
}
