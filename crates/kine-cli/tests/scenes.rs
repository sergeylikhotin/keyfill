//! Сквозные тесты сцен: золотые дампы и поведение must/else, signal.untracked.
//! Обновить снапшоты: `KINE_UPDATE_SNAPSHOTS=1 cargo test -p kine-cli`.
//!
//! Сцены с шрифтами подключают Inter из assets и выключают системные шрифты,
//! поэтому дамп не зависит от машины.

use std::path::{Path, PathBuf};
use std::sync::Once;

use kine_cli::Rendered;
use kine_core::diag::Level;
use kine_core::{EvalOptions, Mode};
use kine_js::RunOptions;

fn home() -> Option<PathBuf> {
    let home = std::path::absolute(kine_cli::kine_home()).unwrap();
    if !home.join("node_modules/esbuild").exists() {
        eprintln!("skip: нет node_modules/esbuild, выполни npm install");
        return None;
    }
    // Пути в дампе относительны рабочего каталога, снапшоты сняты из корня репозитория.
    static CWD: Once = Once::new();
    CWD.call_once(|| std::env::set_current_dir(&home).unwrap());
    Some(home)
}

fn run(entry: &str, mode: Mode) -> Rendered {
    kine_cli::frame(Path::new(entry), &RunOptions::default(), EvalOptions { mode, background: None }).unwrap()
}

fn snapshot(entry: &str, mode: Mode, snap: &str) {
    let Some(home) = home() else { return };
    let first = run(entry, mode);
    let dump = kine_core::dump::dump(&first.scene, &first.frame);
    let second = run(entry, mode);
    assert_eq!(dump, kine_core::dump::dump(&second.scene, &second.frame), "дамп {entry} недетерминирован");

    let path = home.join(snap);
    if std::env::var_os("KINE_UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, &dump).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("нет снапшота {snap}: запусти с KINE_UPDATE_SNAPSHOTS=1"));
    assert_eq!(dump.replace("\r\n", "\n"), expected.replace("\r\n", "\n"), "дамп {entry} разошёлся со снапшотом {snap}");
}

#[test]
fn lower_third_snapshot() {
    snapshot("examples/lower-third/scene.tsx", Mode::Dev, "examples/lower-third/scene.dump.txt");
}

#[test]
fn scoreboard_snapshot() {
    snapshot("examples/scoreboard/scene.tsx", Mode::Dev, "examples/scoreboard/scene.dump.txt");
}

#[test]
fn overflow_is_error_in_dev_with_else_hint() {
    if home().is_none() {
        return;
    }
    let r = run("examples/scoreboard/overflow.tsx", Mode::Dev);
    let d = r.frame.diagnostics.0.iter().find(|d| d.rule == "must.fit").expect("must.fit");
    assert_eq!(d.level, Level::Error);
    assert_eq!(d.node, "Scoreboard#1/away");
    assert!(d.hint.as_deref().unwrap_or("").starts_with("else[0] это исправляет"), "{d}");
    assert!(d.source.as_deref().unwrap_or("").starts_with("examples/scoreboard/Scoreboard.tsx:"), "{d}");
}

#[test]
fn overflow_applies_else_in_air() {
    if home().is_none() {
        return;
    }
    let r = run("examples/scoreboard/overflow.tsx", Mode::Air);
    let diags = &r.frame.diagnostics.0;
    assert!(!diags.iter().any(|d| d.level == Level::Error), "{diags:#?}");
    let away = r.scene.find("Scoreboard#1/away").unwrap();
    let node = r.frame.find(away).unwrap();
    assert_eq!(node.applied_else.as_deref(), Some("else[0]"));
    let (index, _) = node.text.unwrap();
    assert_eq!(r.frame.texts[index].line_count(), 2);
}

#[test]
fn untracked_signal_read_is_reported() {
    if home().is_none() {
        return;
    }
    let r = run("crates/kine-cli/tests/fixtures/untracked.tsx", Mode::Dev);
    let d = r.frame.diagnostics.0.iter().find(|d| d.rule == "signal.untracked").expect("signal.untracked");
    assert_eq!(d.node, "Dot#1");
    assert!(d.source.as_deref().unwrap_or("").contains("untracked.tsx"), "{d}");
}

// ─── Движение ───────────────────────────────────────────────────────────────

use kine_cli::Session;
use kine_core::motion::RelMotion;
use kine_cli::session::{CueArg, parse_cues};

fn session(entry: &str, extra: &[CueArg]) -> Session {
    let source = kine_cli::bundle(Path::new(entry)).unwrap();
    Session::open(&source, &RunOptions::default(), extra).unwrap()
}

fn dev() -> EvalOptions {
    EvalOptions { mode: Mode::Dev, background: None }
}

fn motion_of(s: &Session, frame: &kine_core::Frame, path: &str) -> RelMotion {
    let id = s.scene.find(path).unwrap();
    frame.find(id).and_then(|n| n.anim.as_ref()).map(|a| a.motion).unwrap_or_default()
}

/// Золотой дамп сценария: несколько моментов подряд, агент читает его как раскадровку.
#[test]
fn lower_third_cues_timeline_snapshot() {
    let Some(home) = home() else { return };
    let times = [0.0, 0.1, 0.46, 1.6, 1.8, 2.6, 3.6, 3.76];
    let render = || {
        let mut s = session("examples/lower-third/scene.cues.ts", &[]);
        let mut parts = Vec::new();
        for &t in &times {
            let frame = s.frame_at(t, dev()).unwrap();
            parts.push(kine_core::dump::dump(&s.scene, &frame));
        }
        parts.join("\n")
    };
    let dump = render();
    assert_eq!(dump, render(), "сценарий недетерминирован");
    let snap = "examples/lower-third/scene.cues.dump.txt";
    let path = home.join(snap);
    if std::env::var_os("KINE_UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, &dump).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("нет снапшота {snap}: запусти с KINE_UPDATE_SNAPSHOTS=1"));
    assert_eq!(dump.replace("\r\n", "\n"), expected.replace("\r\n", "\n"), "дамп сценария разошёлся со снапшотом {snap}");
}

/// Кадр зависит только от момента, а не от того, какие кадры рендерились до него.
#[test]
fn frame_does_not_depend_on_path() {
    if home().is_none() {
        return;
    }
    let mut walked = session("examples/lower-third/scene.cues.ts", &[]);
    for i in 0..=90 {
        walked.frame_at(i as f64 / 50.0, dev()).unwrap();
    }
    let a = walked.frame_at(1.84, dev()).unwrap();
    let mut direct = session("examples/lower-third/scene.cues.ts", &[]);
    let b = direct.frame_at(1.84, dev()).unwrap();
    assert_eq!(kine_core::dump::dump(&walked.scene, &a), kine_core::dump::dump(&direct.scene, &b));
    let opts = kine_render::RenderOptions::default();
    assert_eq!(kine_render::render_png(&a, &opts).unwrap(), kine_render::render_png(&b, &opts).unwrap(), "пиксели разошлись");
}

/// stop посреди in: out начинается с текущей прозрачности, а не с объявленной 1.
#[test]
fn stop_during_in_continues_from_current_value() {
    if home().is_none() {
        return;
    }
    let mut s = session("examples/lower-third/scene.tsx", &parse_cues("5f stop").unwrap());
    let frame = s.frame_at(0.1, dev()).unwrap();
    let at_stop = motion_of(&s, &frame, "LowerThird#1/panel");
    let frame = s.frame_at(0.12, dev()).unwrap();
    let after = motion_of(&s, &frame, "LowerThird#1/panel");
    assert!(at_stop.opacity > 0.2 && at_stop.opacity < 0.9, "{at_stop:?}");
    assert!(after.opacity < at_stop.opacity && after.opacity > at_stop.opacity - 0.2, "скачок прозрачности: {at_stop:?} → {after:?}");
    // Сдвиг из in продолжает свою пружину: out его не трогает.
    assert!(after.dx > at_stop.dx, "{at_stop:?} → {after:?}");
    let gone = s.frame_at(0.4, dev()).unwrap();
    assert!(gone.removed && gone.roots.is_empty());
}

/// update меняет данные; изменение look анимируется через transition, а when уводит узел через out.
#[test]
fn update_runs_transition_and_out() {
    if home().is_none() {
        return;
    }
    let mut s = session("examples/lower-third/scene.cues.ts", &[]);
    let panel = s.scene.find("LowerThird#1/panel").unwrap();
    let dot = s.scene.find("LowerThird#1/dot").unwrap();
    let mid = s.frame_at(1.62, dev()).unwrap();
    let node = mid.find(panel).unwrap();
    let anim = node.anim.as_ref().expect("переход идёт");
    assert!(anim.notes.iter().any(|n| n.starts_with("transition background")), "{:?}", anim.notes);
    let bg = node.visual.background.unwrap();
    // Между surfaceLive (#2A0F16) и surface (#101418).
    assert!(bg.r < 0x2A as f32 / 255.0 && bg.r > 0x10 as f32 / 255.0, "{bg:?}");
    // Точка уже скрыта (when=false), но ещё уходит.
    let leaving = mid.find(dot).expect("точка рисуется во время out");
    assert!(leaving.anim.as_ref().unwrap().notes[0].starts_with("out"));
    let later = s.frame_at(1.8, dev()).unwrap();
    assert!(later.find(dot).is_none());
}
