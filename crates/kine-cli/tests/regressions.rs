//! Регрессии по критическому ревью: сцены воспроизведения лежат в `tests/fixtures/review`.
//! Каждый тест закрепляет исправленное поведение, а не снимок дампа.

use std::path::{Path, PathBuf};
use std::sync::Once;

use kine_cli::Session;
use kine_cli::ops::{self, Job};
use kine_cli::session::{CueArg, parse_cues};
use kine_core::motion::{Note, ScenePhase};
use kine_core::timeline::RunState;
use kine_core::{EvalOptions, Frame, Mode};
use kine_js::RunOptions;

const REVIEW: &str = "crates/kine-cli/tests/fixtures/review";

fn home() -> Option<PathBuf> {
    let home = std::path::absolute(kine_cli::kine_home()).unwrap();
    if !home.join("node_modules/esbuild").exists() {
        eprintln!("skip: нет node_modules/esbuild, выполни npm install");
        return None;
    }
    static CWD: Once = Once::new();
    CWD.call_once(|| std::env::set_current_dir(&home).unwrap());
    Some(home)
}

fn session(entry: &str, cues: &str) -> Option<Session> {
    home()?;
    let extra: Vec<CueArg> = if cues.is_empty() { Vec::new() } else { parse_cues(cues).unwrap() };
    let bundle = kine_cli::bundle(Path::new(entry)).unwrap();
    Some(Session::open(&bundle, &RunOptions::default(), &extra).unwrap())
}

fn dev() -> EvalOptions {
    EvalOptions { mode: Mode::Dev, background: None }
}

/// Прозрачность слоя движения и τ таймлайна у узла.
fn opacity_and_tau(s: &Session, frame: &Frame, path: &str) -> (f32, Option<f64>) {
    let id = s.scene.find(path).unwrap();
    let anim = frame.find(id).and_then(|n| n.anim.as_ref()).unwrap();
    let tau = anim.notes.iter().find_map(|n| if let Note::Timeline { tau, .. } = n { Some(*tau) } else { None });
    (anim.motion.opacity, tau)
}

#[test]
fn date_is_deterministic_and_utc() {
    let Some(mut s) = session(&format!("{REVIEW}/date.tsx"), "") else { return };
    let frame = s.frame_at(0.0, dev()).unwrap();
    let text = kine_core::dump::dump(&s.scene, &frame);
    assert!(text.contains("1970-01-01T00:00:00.000Z 0 Thu, 01 Jan 1970 00:00:00 GMT"), "{text}");
}

#[test]
fn pingpong_follows_keys_after_segment() {
    // Цикл opacity 1→0 на τ 1..2 туда-обратно; в 1.5 с команда flash играет сегмент и возвращается.
    let Some(mut s) = session(&format!("{REVIEW}/pingpong.cues.ts"), "") else { return };
    for t in [4.2, 5.0, 5.3, 6.1] {
        let frame = s.frame_at(t, dev()).unwrap();
        let (opacity, tau) = opacity_and_tau(&s, &frame, "Card#1/body");
        let tau = tau.unwrap();
        assert!((opacity as f64 - (2.0 - tau)).abs() < 0.02, "t={t}: opacity {opacity} при τ={tau}");
    }
}

#[test]
fn reentered_node_starts_from_declared_values() {
    // out сдвигает узел на 40 вниз; после повторного появления сдвига быть не должно.
    let Some(mut s) = session(&format!("{REVIEW}/reenter.cues.ts"), "") else { return };
    let frame = s.frame_at(3.0, dev()).unwrap();
    let id = s.scene.find("stage/box[0]/box[0]").unwrap();
    let motion = frame.find(id).and_then(|n| n.anim.as_ref()).map(|a| a.motion).unwrap_or_default();
    assert!(motion.is_identity(), "{motion:?}");
}

#[test]
fn early_next_does_not_cancel_loop() {
    let Some(mut s) = session("examples/scoreboard/scene.tsx", "0 play; 0.3 next; 1.5 next") else { return };
    let frame = s.frame_at(4.0, dev()).unwrap();
    let id = s.scene.find("Scoreboard#1/panel").unwrap();
    let notes = &frame.find(id).unwrap().anim.as_ref().unwrap().notes;
    assert!(matches!(notes[0], Note::Timeline { state: RunState::Loop, .. }), "{notes:?}");
    assert!(s.motion.events_log.iter().any(|e| e.kind == "ignored" && (e.t - 0.3).abs() < 1e-9), "ранний next — событие ignored");
}

#[test]
fn slow_spring_has_no_jump_and_warns() {
    let Some(mut s) = session(&format!("{REVIEW}/spring.tsx"), "") else { return };
    let a = s.frame_at(10.0, dev()).unwrap();
    let b = s.frame_at(10.01, dev()).unwrap();
    let id = s.scene.find("stage/box[0]/box[0]").unwrap();
    let dx = |f: &Frame| f.find(id).and_then(|n| n.anim.as_ref()).map_or(0.0, |a| a.motion.dx);
    assert!((dx(&a) - dx(&b)).abs() < 1.0, "скачок {} → {}", dx(&a), dx(&b));
    assert!(b.diagnostics.0.iter().any(|d| d.rule == "motion.spring-slow"));
}

#[test]
fn presence_layer_is_not_checked_by_must() {
    // Ширина 20→300 в in: посреди появления текст «не помещается», но это не нарушение.
    let Some(_) = home() else { return };
    let job = Job::new(Path::new(&format!("{REVIEW}/presence-must.tsx")), RunOptions::default(), Vec::new(), dev()).unwrap();
    let spec = kine_cli::time::parse("0..2").unwrap();
    let check = ops::check(&job, Some(&spec)).unwrap();
    assert_eq!(check.result.counts.error, 0, "{:?}", check.result.issues);
}

#[test]
fn hold_on_out_is_not_crossing_out() {
    let Some(mut s) = session(&format!("{REVIEW}/out-hold.tsx"), "0 play; 3 stop") else { return };
    assert_eq!(s.marker_time("out").unwrap(), 3.0, "out проходится по stop, а не при остановке на нём");
    s.advance(5.0).unwrap();
    assert_eq!(s.motion.events_log.iter().filter(|e| e.kind == "marker" && e.name == "out").count(), 1);
}

#[test]
fn loop_boundaries_land_on_the_same_side() {
    // Цикл 0.1..0.3: на каждой границе витка кадр показывает начало нового витка.
    let Some(mut s) = session(&format!("{REVIEW}/loop-grid.tsx"), "") else { return };
    for t in [0.5, 1.1, 2.1, 3.1, 4.1] {
        let frame = s.frame_at(t, dev()).unwrap();
        let (_, tau) = opacity_and_tau(&s, &frame, "Card#1/body");
        assert_eq!(tau, Some(0.1), "t={t}");
    }
}

#[test]
fn unsupported_channel_keeps_the_timeline() {
    let Some(mut s) = session(&format!("{REVIEW}/blur.tsx"), "") else { return };
    let frame = s.frame_at(0.2, dev()).unwrap();
    let (opacity, _) = opacity_and_tau(&s, &frame, "Card#1/body");
    assert!(opacity < 1.0, "таймлайн работает, хотя в ключе есть blur");
    assert!(frame.diagnostics.0.iter().any(|d| d.rule == "motion.unsupported"));
}

#[test]
fn second_stop_does_not_cut_the_out() {
    let Some(mut s) = session("examples/lower-third/scene.tsx", "0 play; 1 stop; 1.1 stop") else { return };
    s.frame_at(1.1, dev()).unwrap();
    assert!(matches!(s.motion.scene_phase(), ScenePhase::Stopping { .. }), "{:?}", s.motion.scene_phase());
}
