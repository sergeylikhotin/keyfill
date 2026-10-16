//! Вывод CLI: JSON-конверт со схемой `"keyfill": 1` и текст, собранный из той же структуры.
//!
//! Конверт успеха: `{ keyfill, command, ok: true, exit, entry, at, result, logs, next, repro }`.
//! Конверт ошибки: `{ keyfill, command, ok: false, exit, error: { kind, message, diagnostics } }`.
//! `next` — готовые команды следующего шага, `repro` — строка, которая воспроизводит вызов.

use std::fmt::Write as _;

use keyfill_core::diag::{Diagnostic, Level};
use keyfill_core::dump;
use keyfill_core::look::num;
use keyfill_js::{JsException, LogLine};
use serde::Serialize;

use crate::Failure;
use crate::doctor::{self, Cache, Doctor, Status};
use crate::help::{RuleDoc, Topic};
use crate::scaffold::Created;
use crate::ops::{At, Check, Counts, Filmstrip, Perf, Rendered, Schema, Video};
use crate::suite::{self, Report, SnapStatus};

pub const SCHEMA: u32 = 1;

#[derive(Debug, Serialize)]
pub struct Envelope {
    pub keyfill: u32,
    pub command: String,
    pub ok: bool,
    pub exit: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    /// `keyfill.config.ts`, если он задал значения по умолчанию.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<At>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Box<serde_json::value::RawValue>>,
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

/// Код выхода и описание ошибки: 2 — неверный вызов, 3 — сборка, 4 — исключение JS, 5 — сбой keyfill.
pub fn classify(e: &anyhow::Error) -> (u8, ErrorInfo) {
    let message = format!("{e:#}");
    if let Some(ex) = e.downcast_ref::<JsException>() {
        return (4, ErrorInfo { kind: "js", message, diagnostics: vec![js_diagnostic(ex)] });
    }
    match e.downcast_ref::<Failure>() {
        Some(Failure::Usage(_)) => (2, ErrorInfo { kind: "usage", message, diagnostics: Vec::new() }),
        Some(Failure::Props(errors)) => {
            let diagnostics = errors
                .iter()
                .map(|m| Diagnostic { level: Level::Error, rule: "props.invalid".into(), node: "scene".into(), source: None, message: m.clone(), hint: None, detail: None })
                .collect();
            (2, ErrorInfo { kind: "usage", message, diagnostics })
        }
        Some(Failure::Build(diags)) => (3, ErrorInfo { kind: "build", message, diagnostics: diags.clone() }),
        None => (5, ErrorInfo { kind: "internal", message, diagnostics: Vec::new() }),
    }
}

/// Исключение JS как диагностика `js.exception`: место и стек — уже в исходниках сцены.
fn js_diagnostic(ex: &JsException) -> Diagnostic {
    let mut stack: Vec<String> = ex.stack.iter().map(|f| f.to_string()).collect();
    if ex.hidden > 0 {
        stack.push(format!("… ещё {} в @keyfill/core", ex.hidden));
    }
    Diagnostic {
        level: Level::Error,
        rule: "js.exception".into(),
        node: "js".into(),
        source: ex.source.clone(),
        message: ex.message.clone(),
        hint: None,
        detail: (!stack.is_empty()).then(|| serde_json::json!({ "stack": stack })),
    }
}

pub fn failure(command: &str, entry: Option<&str>, e: &anyhow::Error) -> (u8, Envelope) {
    let (exit, error) = classify(e);
    let next = match (entry, e.downcast_ref::<Failure>()) {
        (Some(entry), Some(Failure::Props(_))) => vec![format!("keyfill schema {}", quote(entry))],
        _ => Vec::new(),
    };
    let env = Envelope {
        keyfill: SCHEMA,
        command: command.into(),
        ok: false,
        exit,
        entry: entry.map(String::from),
        config: None,
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

/// Число со словом в нужной форме: `[кадр, кадра, кадров]` → `1 кадр`, `3 кадра`, `11 кадров`.
fn plural(n: usize, [one, few, many]: [&str; 3]) -> String {
    let word = match (n % 10, n % 100) {
        (1, r) if r != 11 => one,
        (2..=4, r) if !(12..=14).contains(&r) => few,
        _ => many,
    };
    format!("{n} {word}")
}

/// `1 кадр`, `3 кадра`, `231 кадр`, `11 кадров`.
pub fn frames(n: usize) -> String {
    plural(n, ["кадр", "кадра", "кадров"])
}

fn counts(c: &Counts) -> String {
    c.to_string()
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
        let _ = writeln!(out, "ошибки диагностики в кадрах: {}; подробно — keyfill check <сцена> --t {}..{}", v.frames_with_errors, num(v.from as f32), num(v.to as f32));
    }
    out
}

fn prop_line(p: &keyfill_core::scene::PropInfo) -> String {
    match &p.default {
        Some(v) => format!("{}: {} = {v}", p.name, p.ty),
        None if p.required => format!("{}: {} (обязательный)", p.name, p.ty),
        None => format!("{}: {}", p.name, p.ty),
    }
}

pub fn schema_text(s: &Schema) -> String {
    let mut out = format!("stage {}×{} @{}fps\n", num(s.stage.size[0]), num(s.stage.size[1]), num(s.stage.fps));
    if let Some(c) = &s.channel {
        let catalog = if c.catalog.is_empty() { "пуст".to_string() } else { c.catalog.join(", ") };
        let _ = writeln!(out, "channel  каталог: {catalog}");
    } else if s.props.is_empty() {
        out.push_str("props  — (сцена без данных)\n");
    } else {
        out.push_str("props");
        out.push('\n');
        for p in &s.props {
            let _ = writeln!(out, "  {}", prop_line(p));
        }
    }
    out.push_str("components\n");
    for c in &s.components {
        let inst = match (&s.channel, c.instances.is_empty()) {
            (Some(_), true) => "на слоях нет".to_string(),
            (None, true) => "не используется".to_string(),
            _ => c.instances.join(", "),
        };
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
                let kind = if l.pingpong { " pingpong" } else { "" };
                let count = l.count.map(|n| format!(" ×{n}")).unwrap_or_default();
                let _ = writeln!(out, "    loop     {}..{}{kind}{count}", l.from, l.to);
            }
        }
    }
    if !s.cues.is_empty() {
        let _ = writeln!(out, "cues  {}", s.cues.join(" · "));
    }
    if let Some(c) = &s.channel
        && !c.journal.is_empty()
    {
        let _ = writeln!(out, "journal  {}", c.journal.join(" · "));
    }
    out
}

pub fn perf_text(p: &Perf) -> String {
    let mut out = format!(
        "perf {}..{} с · {} · бюджет {} мс ({} fps) · {} сборка · запуск {} мс\n",
        num(p.from as f32),
        num(p.to as f32),
        frames(p.frames),
        p.budget_ms,
        p.fps,
        p.build,
        p.mount_ms
    );
    out.push_str("  стадия      mean     p50     p95     max  мс\n");
    for s in &p.stages {
        let _ = writeln!(out, "  {:<8} {:>7.3} {:>7.3} {:>7.3} {:>7.3}", s.stage, s.mean_ms, s.p50_ms, s.p95_ms, s.max_ms);
    }
    let _ = writeln!(out, "дольше бюджета: {}", frames(p.over_budget));
    let slow: Vec<String> = p.slowest.iter().map(|s| format!("кадр {} ({} с) {} мс", s.at.frame, num(s.at.t as f32), s.ms)).collect();
    if !slow.is_empty() {
        let _ = writeln!(out, "самые долгие: {}", slow.join(" · "));
    }
    for d in &p.diagnostics {
        let _ = writeln!(out, "{d}");
    }
    out
}

pub fn rule_text(r: &RuleDoc) -> String {
    format!("{} — {} · {}\n\n{}\n", r.rule, r.level, r.title, r.body)
}

pub fn rules_text(rules: &Vec<RuleDoc>) -> String {
    let width = rules.iter().map(|r| r.rule.chars().count()).max().unwrap_or(0);
    let mut out = String::new();
    for r in rules {
        let _ = writeln!(out, "{:<width$}  {:<5}  {}", r.rule, r.level, r.title);
    }
    out.push_str("\nподробно: keyfill explain <правило>\n");
    out
}

pub fn topic_text(t: &Topic) -> String {
    format!("{} — {}\n\n{}\n", t.topic, t.title, t.body)
}

pub fn topics_text(topics: &Vec<Topic>) -> String {
    let width = topics.iter().map(|t| t.topic.len()).max().unwrap_or(0);
    let mut out = String::new();
    for t in topics {
        let _ = writeln!(out, "{:<width$}  {}", t.topic, t.title);
    }
    out.push_str("\nтема: keyfill docs <тема>; правила диагностики: keyfill explain\n");
    out
}

pub fn test_text(r: &Report) -> String {
    let mut out = String::new();
    let mut file = "";
    for c in &r.cases {
        if c.file != file {
            file = &c.file;
            let _ = writeln!(out, "{file}");
        }
        let times: Vec<&str> = c.moments.iter().map(|m| m.t.as_str()).collect();
        let name = if c.name.is_empty() { "(файл)".to_string() } else { c.name.clone() };
        let _ = writeln!(out, "  {}  {name}  @ {}", if c.ok { "ok  " } else { "FAIL" }, times.join(", "));
        if let Some(e) = &c.error {
            error_lines(&mut out, e);
        }
        for m in &c.moments {
            if let Some(e) = &m.error {
                let _ = writeln!(out, "      @{}:", m.t);
                error_lines(&mut out, e);
            }
            for check in &m.checks {
                check_lines(&mut out, &m.t, check);
            }
        }
    }
    let k = &r.counts;
    let mut summary = format!("{} · {} ok, {} fail", plural(r.files, ["файл", "файла", "файлов"]), k.passed, k.failed);
    if k.written > 0 {
        let _ = write!(summary, ", новых снапшотов: {}", k.written);
    }
    if k.updated > 0 {
        let _ = write!(summary, ", перезаписано снапшотов: {}", k.updated);
    }
    let _ = writeln!(out, "{summary}");
    out
}

fn error_lines(out: &mut String, e: &ErrorInfo) {
    if e.diagnostics.is_empty() {
        let _ = writeln!(out, "      {}", e.message.replace('\n', "\n      "));
    }
    for d in &e.diagnostics {
        let _ = writeln!(out, "      {}", d.to_string().replace('\n', "\n      "));
    }
}

fn check_lines(out: &mut String, t: &str, check: &suite::Check) {
    match check {
        suite::Check::Clean { ok: false, issues } => {
            for i in issues {
                let _ = writeln!(out, "      @{t} {}", i.diagnostic.to_string().replace('\n', "\n        "));
            }
        }
        suite::Check::Clean { .. } => {}
        suite::Check::Dump { status, file, diff, more } => {
            snap_line(out, t, "dump", *status, file);
            for d in diff {
                let node = d.node.as_deref().unwrap_or("");
                match (d.op, &d.old, &d.new) {
                    ("~", Some(old), Some(new)) => {
                        let _ = writeln!(out, "        ~ {node}\n          - {old}\n          + {new}");
                    }
                    (op, old, new) => {
                        let body = new.as_deref().or(old.as_deref()).unwrap_or("");
                        let _ = writeln!(out, "        {op} {}{body}", if node.is_empty() { String::new() } else { format!("{node}  ") });
                    }
                }
            }
            if *more > 0 {
                let _ = writeln!(out, "        … ещё {more} строк разницы");
            }
        }
        suite::Check::Png { status, file, pixels } => {
            snap_line(out, t, "png", *status, file);
            if let Some(p) = pixels {
                let share = p.differing as f64 / p.total.max(1) as f64 * 100.0;
                let mut line = format!("        отличаются {} из {} пикселей ({share:.3}%)", p.differing, p.total);
                if let Some([x, y, w, h]) = p.bbox {
                    let _ = write!(line, " в {x},{y} {w}×{h}");
                }
                if let Some((a, b)) = p.size {
                    let _ = write!(line, "; размер {}×{} → {}×{}", a[0], a[1], b[0], b[1]);
                }
                let _ = writeln!(out, "{line}\n        новый кадр {} · маска {}", p.actual, p.mask);
            }
        }
    }
}

fn snap_line(out: &mut String, t: &str, kind: &str, status: SnapStatus, file: &str) {
    let what = match status {
        SnapStatus::Same => return,
        SnapStatus::Written => "записан",
        SnapStatus::Updated => "перезаписан",
        SnapStatus::Changed => "изменился",
    };
    let _ = writeln!(out, "      @{t} {kind} {what}: {file}");
}

pub fn new_text(c: &Created) -> String {
    let mut out = format!("создана сцена {} (шаблон {})
", c.dir, c.template);
    for f in &c.files {
        let _ = writeln!(out, "  + {f}");
    }
    if !c.kept.is_empty() {
        let _ = writeln!(out, "  уже были: {}", c.kept.join(", "));
    }
    out
}

pub fn doctor_text(d: &Doctor) -> String {
    let mut out = format!("keyfill {} · {} · {}
", d.version, d.build, d.platform);
    for c in &d.checks {
        let mark = match c.status {
            Status::Ok => "ok  ",
            Status::Warn => "warn",
            Status::Error => "FAIL",
        };
        let _ = writeln!(out, "  {mark}  {:<11} {}", c.name, c.detail);
        if let Some(h) = &c.hint {
            let _ = writeln!(out, "        {:<11} → {h}", "");
        }
    }
    out
}

pub fn cache_text(c: &Cache) -> String {
    let Some(dir) = &c.dir else { return "кэш выключен (KEYFILL_NO_CACHE)
".into() };
    let what = format!("{} · {}", doctor::files(c.files), doctor::bytes(c.bytes));
    if c.cleaned { format!("удалён {dir} ({what})
") } else { format!("{dir} · {what}
") }
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
