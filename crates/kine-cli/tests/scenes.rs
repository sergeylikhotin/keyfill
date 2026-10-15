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
