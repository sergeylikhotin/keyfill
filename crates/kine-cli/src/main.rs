use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use kine_core::diag::Level;
use kine_core::{EvalOptions, Mode};
use kine_core::look::parse_color;
use kine_js::RunOptions;
use kine_render::RenderOptions;

const USAGE: &str = "\
kine — рендер TSX-сцен без браузера

  kine render <scene.tsx> [-o out.png] [--scale N] [--dump]
  kine dump <scene.tsx>               текстовое дерево сцены с геометрией и диагностиками
  kine inspect <scene.tsx> <path>     один узел: значения, их происхождение, исходник
  kine check <scene.tsx> [--json]     только диагностики; код выхода 1 при ошибках

Общие флаги:
  --mode dev|air   dev (по умолчанию): нарушение must — ошибка; air: применяются шаги else
  --bg #RRGGBB     подложка: рисуется под сценой и учитывается в проверке контраста
  --seed N         сид Math.random
  --now MS         значение Date.now";

struct Args {
    rest: Vec<String>,
    flags: Vec<(String, Option<String>)>,
}

impl Args {
    fn parse(raw: Vec<String>) -> Self {
        let valued = ["-o", "--scale", "--bg", "--seed", "--now", "--mode"];
        let mut rest = Vec::new();
        let mut flags = Vec::new();
        let mut it = raw.into_iter();
        while let Some(a) = it.next() {
            if valued.contains(&a.as_str()) {
                let v = it.next();
                flags.push((a, v));
            } else if a.starts_with('-') {
                flags.push((a, None));
            } else {
                rest.push(a);
            }
        }
        Self { rest, flags }
    }

    fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|(k, _)| k == name)
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.flags.iter().find(|(k, _)| k == name).and_then(|(_, v)| v.as_deref())
    }

    fn entry(&self) -> Result<PathBuf> {
        self.rest.get(1).map(PathBuf::from).context("не указан файл сцены")
    }
}

fn run_options(args: &Args) -> Result<RunOptions> {
    let mut opts = RunOptions::default();
    if let Some(s) = args.value("--seed") {
        opts.seed = s.parse().context("--seed: ожидается целое")?;
    }
    if let Some(s) = args.value("--now") {
        opts.now_ms = s.parse().context("--now: ожидается число миллисекунд")?;
    }
    Ok(opts)
}

fn parse_hex(s: &str) -> Result<kine_core::look::Rgba> {
    let h = s.trim_start_matches('#');
    if h.len() != 6 {
        bail!("--bg: ожидается #RRGGBB");
    }
    let n = |i: usize| u8::from_str_radix(&h[i..i + 2], 16);
    let json = serde_json::json!({ "r": n(0)?, "g": n(2)?, "b": n(4)? });
    parse_color(&json).map_err(anyhow::Error::msg)
}

fn main() -> ExitCode {
    let args = Args::parse(std::env::args().skip(1).collect());
    match run(&args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn run(args: &Args) -> Result<ExitCode> {
    let Some(cmd) = args.rest.first().map(String::as_str) else {
        println!("{USAGE}");
        return Ok(ExitCode::SUCCESS);
    };
    if matches!(cmd, "help" | "-h" | "--help") {
        println!("{USAGE}");
        return Ok(ExitCode::SUCCESS);
    }

    let entry = args.entry()?;
    let background = args.value("--bg").map(parse_hex).transpose()?;
    let mode = match args.value("--mode") {
        None | Some("dev") => Mode::Dev,
        Some("air") => Mode::Air,
        Some(other) => bail!("--mode: ожидается dev или air, получено {other}"),
    };
    let rendered = kine_cli::frame(&entry, &run_options(args)?, EvalOptions { mode, background })?;
    for log in &rendered.logs {
        eprintln!("js {}: {}", log.level, log.message);
    }
    let (scene, frame) = (&rendered.scene, &rendered.frame);

    match cmd {
        "render" => {
            let out = args.value("-o").map(PathBuf::from).unwrap_or_else(|| entry.with_extension("png"));
            let opts = RenderOptions {
                scale: args.value("--scale").map(str::parse).transpose().context("--scale: ожидается число")?.unwrap_or(1.0),
                background,
            };
            let png = kine_render::render_png(frame, &opts)?;
            std::fs::write(&out, png).with_context(|| format!("не удалось записать {}", out.display()))?;
            if args.flag("--dump") {
                print!("{}", kine_core::dump::dump(scene, frame));
            }
            eprintln!("written {}", out.display());
        }
        "dump" => print!("{}", kine_core::dump::dump(scene, frame)),
        "inspect" => {
            let path = args.rest.get(2).context("не указан адрес узла, например LowerThird#1/title")?;
            match kine_core::dump::inspect(scene, frame, path) {
                Some(text) => print!("{text}"),
                None => bail!("узел {path} не найден; адреса узлов есть в `kine dump`"),
            }
        }
        "check" => {
            let diags = &frame.diagnostics.0;
            if args.flag("--json") {
                println!("{}", serde_json::to_string_pretty(diags)?);
            } else {
                for d in diags {
                    println!("{d}");
                }
            }
            if diags.iter().any(|d| d.level == Level::Error) {
                return Ok(ExitCode::from(1));
            }
        }
        other => bail!("неизвестная команда {other}\n\n{USAGE}"),
    }
    Ok(ExitCode::SUCCESS)
}
