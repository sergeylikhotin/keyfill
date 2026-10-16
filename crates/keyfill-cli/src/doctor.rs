//! `keyfill doctor` — проверка окружения, `keyfill cache [clean]` — кэш бандлов.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::Result;
use keyfill_core::EvalOptions;
use keyfill_js::RunOptions;
use serde::Serialize;

use crate::{CoreSource, shown};

/// Минимальная версия node: `bundle.mjs` — ES-модуль с `await` верхнего уровня.
const NODE_MAJOR: u32 = 18;

#[derive(Debug, Serialize)]
pub struct Doctor {
    pub version: &'static str,
    pub build: &'static str,
    pub platform: String,
    pub checks: Vec<Probe>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    /// Работает, но не всё: без ffmpeg нет видео.
    Warn,
    /// keyfill не соберёт сцену.
    Error,
}

#[derive(Debug, Serialize)]
pub struct Probe {
    pub name: &'static str,
    pub status: Status,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl Doctor {
    pub fn errors(&self) -> bool {
        self.checks.iter().any(|c| c.status == Status::Error)
    }
}

fn probe(name: &'static str, status: Status, detail: impl Into<String>, hint: Option<&str>) -> Probe {
    Probe { name, status, detail: detail.into(), hint: hint.map(String::from) }
}

/// Первая строка вывода программы, если она запустилась и завершилась успешно.
fn version_of(program: &str, arg: &str) -> Option<String> {
    let out = Command::new(program).arg(arg).stdin(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or("").trim().to_string())
}

/// Поле `version` из `package.json` каталога.
fn package_version(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("package.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v["version"].as_str().map(String::from)
}

pub fn run() -> Doctor {
    let mut checks = Vec::new();

    match version_of("node", "--version") {
        Some(v) => {
            let major = v.trim_start_matches('v').split('.').next().and_then(|m| m.parse::<u32>().ok()).unwrap_or(0);
            if major >= NODE_MAJOR {
                checks.push(probe("node", Status::Ok, v, None));
            } else {
                checks.push(probe("node", Status::Error, format!("{v}: нужна {NODE_MAJOR} или новее"), Some("обнови node: https://nodejs.org")));
            }
        }
        None => checks.push(probe("node", Status::Error, "node не найден в PATH", Some("установи node 18+: им keyfill собирает сцены через esbuild"))),
    }

    let (core, source) = crate::core_dir();
    let from = match source {
        CoreSource::Env => "KEYFILL_CORE",
        CoreSource::NodeModules => "node_modules",
        CoreSource::Repo => "репозиторий keyfill",
    };
    let core_ok = core.join("bundle.mjs").is_file() && core.join("src/index.ts").is_file();
    if core_ok {
        let version = package_version(&core);
        let mismatch = version.as_deref().is_some_and(|v| v != env!("CARGO_PKG_VERSION"));
        let detail = format!("{} ({from}{})", shown(&core), version.as_deref().map(|v| format!(", {v}")).unwrap_or_default());
        if mismatch {
            let hint = format!("версия keyfill {} — поставь @keyfill/core той же версии", env!("CARGO_PKG_VERSION"));
            checks.push(probe("@keyfill/core", Status::Warn, detail, Some(&hint)));
        } else {
            checks.push(probe("@keyfill/core", Status::Ok, detail, None));
        }
    } else {
        checks.push(probe("@keyfill/core", Status::Error, format!("нет bundle.mjs и src в {} ({from})", shown(&core)), Some("npm i -D @keyfill/cli в проекте или KEYFILL_CORE=<каталог @keyfill/core>")));
    }

    match crate::esbuild_dir(&core) {
        Some(dir) => checks.push(probe("esbuild", Status::Ok, format!("{} {}", shown(&dir), package_version(&dir).unwrap_or_default()), None)),
        None => checks.push(probe("esbuild", Status::Error, "не найден в node_modules над @keyfill/core", Some("npm install"))),
    }

    // Весь конвейер на встроенной сцене: сборка, QuickJS, раскладка.
    if checks.iter().all(|c| c.status != Status::Error) {
        checks.push(pipeline());
    }

    match version_of("ffmpeg", "-version") {
        Some(v) => checks.push(probe("ffmpeg", Status::Ok, v, None)),
        None => checks.push(probe("ffmpeg", Status::Warn, "ffmpeg не найден в PATH", Some("нужен только для keyfill video в .mov, .webm, .mp4; PNG-кадры пишутся без него"))),
    }

    let cwd = std::env::current_dir().unwrap_or_default();
    match crate::config::find(&cwd) {
        None => checks.push(probe("config", Status::Ok, "keyfill.config.ts нет: значения по умолчанию", None)),
        Some(file) => match crate::config::read(&file) {
            Ok(_) => checks.push(probe("config", Status::Ok, shown(&file), None)),
            Err(e) => checks.push(probe("config", Status::Error, format!("{}: {e:#}", shown(&file)), Some("keyfill docs config"))),
        },
    }

    let ts = cwd.ancestors().map(|d| d.join("node_modules/typescript")).find(|d| d.join("package.json").is_file());
    match ts {
        Some(dir) => checks.push(probe("typescript", Status::Ok, package_version(&dir).unwrap_or_default(), None)),
        None => checks.push(probe("typescript", Status::Warn, "typescript не найден в node_modules", Some("npm i -D typescript: tsc проверяет пропсы, части и сценарии до рендера"))),
    }

    match crate::bundle::cache_dir() {
        None => checks.push(probe("cache", Status::Ok, "выключен (KEYFILL_NO_CACHE)", None)),
        Some(dir) => {
            let usage = usage(&dir);
            checks.push(probe("cache", Status::Ok, format!("{} · {} · {}", shown(&dir), files(usage.files), bytes(usage.bytes)), None));
        }
    }

    Doctor {
        version: env!("CARGO_PKG_VERSION"),
        build: if cfg!(debug_assertions) { "debug" } else { "release" },
        platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        checks,
    }
}

const PROBE_SCENE: &str = "import { Box, Text, defineScene } from '@keyfill/core';\n\
export default defineScene({ stage: { ref: [320, 180] }, fps: 25, systemFonts: false, root: () => <Box size={{ w: 100, h: 40 }}><Text>keyfill</Text></Box> });\n";

/// Сборка и кадр встроенной сцены во временном каталоге, без кэша.
fn pipeline() -> Probe {
    let start = Instant::now();
    let dir = std::env::temp_dir().join(format!("keyfill-doctor-{}", std::process::id()));
    let run = || -> Result<()> {
        std::fs::create_dir_all(&dir)?;
        let entry = dir.join("probe.tsx");
        std::fs::write(&entry, PROBE_SCENE)?;
        let bundle = crate::bundle::build(&entry)?;
        let mut session = crate::Session::open(&bundle, &RunOptions::default(), &[])?;
        session.frame_at(0.0, EvalOptions::default())?;
        Ok(())
    };
    let result = run();
    let _ = std::fs::remove_dir_all(&dir);
    match result {
        Ok(()) => probe("pipeline", Status::Ok, format!("сборка и кадр пробной сцены за {} мс", start.elapsed().as_millis()), None),
        Err(e) => probe("pipeline", Status::Error, format!("{e:#}"), Some("пробная сцена не собралась: проверь node и esbuild выше")),
    }
}

// ─── cache ──────────────────────────────────────────────────────────────────

#[derive(Debug, Default, Serialize)]
pub struct Cache {
    /// Каталог кэша; `None` — кэш выключен (`KEYFILL_NO_CACHE`).
    pub dir: Option<String>,
    pub files: usize,
    pub bytes: u64,
    /// `keyfill cache clean`: кэш удалён.
    pub cleaned: bool,
}

struct Usage {
    files: usize,
    bytes: u64,
}

fn usage(dir: &Path) -> Usage {
    let mut u = Usage { files: 0, bytes: 0 };
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if let Some(m) = e.metadata().ok().filter(|m| m.is_file()) {
            u.files += 1;
            u.bytes += m.len();
        }
    }
    u
}

pub fn cache(clean: bool) -> Result<Cache> {
    let Some(dir) = crate::bundle::cache_dir() else { return Ok(Cache::default()) };
    let Usage { files, bytes } = usage(&dir);
    if clean && dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| crate::usage(format!("не удалось удалить {}: {e}", shown(&dir))))?;
    }
    Ok(Cache { dir: Some(shown(&dir)), files, bytes, cleaned: clean })
}

pub fn files(n: usize) -> String {
    format!("файлов: {n}")
}

pub fn bytes(n: u64) -> String {
    match n {
        n if n >= 1 << 20 => format!("{:.1} МБ", n as f64 / (1 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.1} КБ", n as f64 / 1024.0),
        n => format!("{n} Б"),
    }
}
