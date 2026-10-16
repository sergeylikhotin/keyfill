//! Вывод CLI: JSON-конверт со схемой `"kine": 1` и текст, собранный из той же структуры.
//!
//! Конверт успеха: `{ kine, command, ok: true, exit, entry, at, result, logs, next, repro }`.
//! Конверт ошибки: `{ kine, command, ok: false, exit, error: { kind, message, diagnostics } }`.
//! `next` — готовые команды следующего шага, `repro` — строка, которая воспроизводит вызов.

use std::fmt::Write as _;

use kine_core::diag::{Diagnostic, Level};
use kine_core::dump;
use kine_core::look::num;
use kine_js::{JsException, LogLine};
use serde::Serialize;
use serde_json::Value;

use crate::Failure;
use crate::bundle::BuildErrors;
use crate::ops::{At, Check, Counts, Filmstrip, Rendered, Schema, Video};

pub const SCHEMA: u32 = 1;

#[derive(Debug, Serialize)]
pub struct Envelope {
    pub kine: u32,
    pub command: String,
    pub ok: bool,
    pub exit: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<At>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorInfo>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub logs: Vec<Log>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub next: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repro: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Log {
    pub level: String,
    pub message: String,
}

impl From<&LogLine> for Log {
    fn from(l: &LogLine) -> Log {
        Log { level: l.level.clone(), message: l.message.clone() }
    }
}

#[derive(Debug, Serialize)]
pub struct ErrorInfo {
    /// `usage`, `build`, `js`, `internal`.
    pub kind: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

/// Код выхода и описание ошибки: 2 — неверный вызов, 3 — сборка, 4 — исключение JS, 5 — сбой kine.
pub fn classify(e: &anyhow::Error) -> (u8, ErrorInfo) {
    let message = message(e);
    if let Some(build) = e.downcast_ref::<BuildErrors>() {
        return (3, ErrorInfo { kind: "build", message: build.to_string(), diagnostics: build.0.clone() });
    }
    if let Some(ex) = e.downcast_ref::<JsException>() {
        return (4, ErrorInfo { kind: "js", message, diagnostics: vec![js_diagnostic(&ex.0)] });
    }
    match e.downcast_ref::<Failure>() {
        Some(Failure::Usage) => (2, ErrorInfo { kind: "usage", message, diagnostics: Vec::new() }),
        Some(Failure::Build) => (3, ErrorInfo { kind: "build", message, diagnostics: Vec::new() }),
        None => (5, ErrorInfo { kind: "internal", message, diagnostics: Vec::new() }),
    }
}

/// Исключение JS как диагностика `js.exception`: место — первый кадр стека в исходнике сцены.
fn js_diagnostic(text: &str) -> Diagnostic {
    let mut lines = text.lines();
    let first = lines.next().unwrap_or_default();
    let message = first.strip_prefix("JS: ").unwrap_or(first).to_string();
    let source = text.lines().find_map(|l| {
        let l = l.trim();
        let loc = l.strip_prefix("at ")?;
        // `at f (file:1:2)` или `at file:1:2` (место KineError).
        let loc = match (loc.rfind('('), loc.ends_with(')')) {
            (Some(i), true) => &loc[i + 1..loc.len() - 1],
            _ => loc,
        };
        loc.contains(':').then(|| loc.to_string())
    });
    let stack: Vec<&str> = text.lines().skip(1).map(str::trim).filter(|l| !l.is_empty()).collect();
    Diagnostic {
        level: Level::Error,
        rule: "js.exception".into(),
        node: "js".into(),
        source,
        message,
        hint: None,
        detail: (!stack.is_empty()).then(|| serde_json::json!({ "stack": stack })),
    }
}

/// Числа результата — как в текстовом дампе: до сотых, целые без `.0`. Геометрия считается
/// в f32, и без округления в JSON попадает шум вроде `37.799995`.
pub fn tidy(mut v: Value) -> Value {
    fn walk(v: &mut Value) {
        match v {
            Value::Number(n) if n.is_f64() => {
                let x = (n.as_f64().unwrap_or(0.0) * 100.0).round() / 100.0;
                *v = if x.fract() == 0.0 && x.abs() < 1e15 { Value::from(x as i64) } else { Value::from(x) };
            }
            Value::Array(a) => a.iter_mut().for_each(walk),
            Value::Object(o) => o.values_mut().for_each(walk),
            _ => {}
        }
    }
    walk(&mut v);
    v
}

/// Текст ошибки без меток класса: класс виден по коду выхода и `error.kind`.
pub fn message(e: &anyhow::Error) -> String {
    // Метка класса — контекст `Failure`; в цепочке он виден только как текст.
    let labels = [Failure::Usage.to_string(), Failure::Build.to_string()];
    let parts: Vec<String> = e.chain().map(|c| c.to_string()).filter(|s| !labels.contains(s)).collect();
    parts.join(": ")
}

pub fn failure(command: &str, entry: Option<&str>, e: &anyhow::Error) -> (u8, Envelope) {
    let (exit, error) = classify(e);
    let next = match entry {
        Some(entry) if error.message.starts_with("пропсы сцены") => vec![format!("kine schema {}", quote(entry))],
        _ => Vec::new(),
    };
    let env = Envelope {
        kine: SCHEMA,
        command: command.into(),
        ok: false,
        exit,
        entry: entry.map(String::from),
        at: None,
        result: None,
        error: Some(error),
        logs: Vec::new(),
        next,
        repro: None,
    };
    (exit, env)
}

// ─── Текст ──────────────────────────────────────────────────────────────────

/// `1 кадр`, `3 кадра`, `231 кадр`, `11 кадров`.
pub fn frames(n: usize) -> String {
    let word = match (n % 10, n % 100) {
        (1, r) if r != 11 => "кадр",
        (2..=4, r) if !(12..=14).contains(&r) => "кадра",
        _ => "кадров",
    };
    format!("{n} {word}")
}

fn counts(c: &Counts) -> String {
    format!("{} error, {} warn, {} info", c.error, c.warn, c.info)
}

fn at(a: &At) -> String {
    format!("t={}s кадр {}", num(a.t as f32), a.frame)
}

pub fn render_text(r: &Rendered) -> String {
    let mut out = format!("written {} {}×{} · {} · {}\n", r.file, r.size[0], r.size[1], at(&r.at), counts(&r.counts));
    for d in &r.diagnostics {
        if d.level != Level::Info {
            let _ = writeln!(out, "{d}");
        }
    }
    if let Some(d) = &r.dump {
        out.push_str(&dump::text(d));
    }
    out
}

pub fn check_text(c: &Check) -> String {
    let mut out = format!("check {}..{} с · {} · {}\n", num(c.from as f32), num(c.to as f32), frames(c.frames), counts(&c.counts));
    if c.issues.is_empty() {
        out.push_str("нарушений нет\n");
    }
    for i in &c.issues {
        let _ = writeln!(out, "{}", i.diagnostic);
        let span = if i.frames == 1 {
            format!("кадр {} ({} с)", i.first.frame, num(i.first.t as f32))
        } else {
            format!("кадры {}..{} ({}..{} с), всего {}", i.first.frame, i.last.frame, num(i.first.t as f32), num(i.last.t as f32), frames(i.frames))
        };
        let _ = writeln!(out, "    {span}");
    }
    out
}

pub fn filmstrip_text(f: &Filmstrip) -> String {
    let span = match (f.frames.first(), f.frames.last()) {
        (Some(a), Some(b)) => format!("{}..{} с", num(a.t as f32), num(b.t as f32)),
        _ => String::new(),
    };
    let mut out = format!("written {} ({}, {span}) · {}\n", f.file, frames(f.frames.len()), counts(&f.counts));
    for d in &f.dumps {
        out.push_str(&dump::text(d));
        out.push('\n');
    }
    out
}

pub fn video_text(v: &Video) -> String {
    let mut out = format!("written {} ({}, {}..{} с, {})\n", v.file, frames(v.frames), num(v.from as f32), num(v.to as f32), v.codec);
    if v.frames_with_errors > 0 {
        let _ = writeln!(out, "ошибки диагностики в кадрах: {}; подробно — kine check <сцена> --t {}..{}", v.frames_with_errors, num(v.from as f32), num(v.to as f32));
    }
    out
}

fn prop_line(p: &kine_core::scene::PropInfo) -> String {
    match &p.default {
        Some(v) => format!("{}: {} = {v}", p.name, p.ty),
        None if p.required => format!("{}: {} (обязательный)", p.name, p.ty),
        None => format!("{}: {}", p.name, p.ty),
    }
}

pub fn schema_text(s: &Schema) -> String {
    let mut out = format!("stage {}×{} @{}fps\n", num(s.stage.size[0]), num(s.stage.size[1]), num(s.stage.fps));
    out.push_str("props");
    if s.props.is_empty() {
        out.push_str("  — (сцена без данных)\n");
    } else {
        out.push('\n');
        for p in &s.props {
            let _ = writeln!(out, "  {}", prop_line(p));
        }
    }
    out.push_str("components\n");
    for c in &s.components {
        let inst = if c.instances.is_empty() { "не используется".to_string() } else { c.instances.join(", ") };
        let _ = writeln!(out, "  {}  [{inst}]", c.info.name);
        if !c.info.props.is_empty() {
            let props: Vec<String> = c.info.props.iter().map(prop_line).collect();
            let _ = writeln!(out, "    props    {}", props.join(", "));
        }
        if !c.info.parts.is_empty() {
            let _ = writeln!(out, "    parts    {}", c.info.parts.join(", "));
        }
        for cmd in &c.info.commands {
            let params: Vec<String> = cmd.params.iter().map(prop_line).collect();
            let _ = writeln!(out, "    command  {}({})", cmd.name, params.join(", "));
        }
        if let Some(t) = &c.timeline {
            let markers: Vec<String> = t.markers.iter().map(|(n, v)| format!("{n} {}", num(*v as f32))).collect();
            let _ = writeln!(out, "    markers  {}", markers.join(" · "));
            if !t.holds.is_empty() {
                let _ = writeln!(out, "    holds    {}", t.holds.join(", "));
            }
            if let Some(l) = &t.repeat {
                let _ = writeln!(out, "    loop     {l}");
            }
        }
    }
    if !s.cues.is_empty() {
        let _ = writeln!(out, "cues  {}", s.cues.join(" · "));
    }
    out
}

/// Аргумент для командной строки в кавычках, если нужно.
pub fn quote(s: &str) -> String {
    // `#` в начале слова — комментарий shell, в середине (`Scoreboard#1`) безопасен.
    if !s.is_empty() && !s.starts_with('#') && s.chars().all(|c| c.is_alphanumeric() || "-_./:=+,@#".contains(c)) {
        s.to_string()
    } else {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    }
}
