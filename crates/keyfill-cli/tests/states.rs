//! Состояния канала (`defineChannel({ props, layers, states })`): переход как разница
//! состояний, данные канала, отказы прямых команд на слоях состояний, разворот уходящей
//! плашки, снимок и воспроизведение журнала.

use std::path::{Path, PathBuf};
use std::sync::Once;

use keyfill_cli::Session;
use keyfill_cli::channel::{AckStatus, Batch, Entry, InstState, Journal};
use keyfill_core::{EvalOptions, Mode};
use keyfill_js::RunOptions;
use keyfill_render::RenderOptions;
use serde_json::{Value, json};

fn home() -> Option<PathBuf> {
    let home = std::path::absolute(keyfill_cli::keyfill_home()).unwrap();
    if !home.join("node_modules/esbuild").exists() {
        eprintln!("skip: нет node_modules/esbuild, выполни npm install");
        return None;
    }
    static CWD: Once = Once::new();
    CWD.call_once(|| std::env::set_current_dir(&home).unwrap());
    Some(home)
}

const MATCH: &str = "examples/match/channel.tsx";
const SHOW: &str = "examples/match/show.journal.jsonl";

fn open(entry: &str, batches: &[Batch]) -> Session {
    let bundle = keyfill_cli::bundle(Path::new(entry)).unwrap();
    Session::open(&bundle, &RunOptions::default(), &[], batches).unwrap()
}

fn dev() -> EvalOptions {
    EvalOptions { mode: Mode::Dev, background: None }
}

fn cmd(v: Value) -> Entry {
    serde_json::from_value(v).unwrap()
}

fn state(name: &str) -> Entry {
    cmd(json!({ "cmd": "state", "name": name }))
}

fn data(props: Value) -> Entry {
    cmd(json!({ "cmd": "update", "props": props }))
}

/// (слой, экземпляр, состояние экземпляра) по снимку канала.
fn layers(s: &Session) -> Vec<(u32, String, InstState)> {
    let c = s.state().unwrap();
    c.layers.iter().flat_map(|(l, slot)| slot.all().map(move |i| (*l, i.name.clone(), i.state))).collect()
}

fn events(s: &Session, kind: &str) -> Vec<String> {
    s.motion.events_log.iter().filter(|e| e.kind == kind).map(|e| format!("{} {}", e.source, e.name)).collect()
}

#[test]
fn state_is_a_diff_of_layers() {
    if home().is_none() {
        return;
    }
    let mut s = open(MATCH, &[]);
    s.command(vec![state("pregame")]).unwrap();
    assert_eq!(layers(&s), [(20, "LowerThird#1".into(), InstState::Playing)]);
    // Название встречи — из данных канала: сменились команды, сменилась плашка.
    s.advance(0.5).unwrap();
    s.command(vec![data(json!({ "away": "Зенит" }))]).unwrap();
    let props = &s.state().unwrap().layers[&20].current.as_ref().unwrap().props;
    assert_eq!(props["name"], "Динамо Москва — Зенит");

    // pregame → live: плашка уходит, табло входит.
    s.advance(1.0).unwrap();
    s.command(vec![state("live")]).unwrap();
    assert_eq!(layers(&s), [(10, "Scoreboard#1".into(), InstState::Playing), (20, "LowerThird#1".into(), InstState::Leaving)]);
    assert_eq!(s.state().unwrap().state.as_deref(), Some("live"));

    // live → interview: табло — тот же экземпляр, гость входит снизу.
    s.advance(2.0).unwrap();
    let acks = s.command(vec![data(json!({ "guest": "Иван Петров", "guestRole": "Тренер", "homeScore": 2 })), state("interview")]).unwrap();
    assert!(acks.iter().all(|a| a.status == AckStatus::Ok), "{acks:?}");
    assert_eq!(layers(&s), [(10, "Scoreboard#1".into(), InstState::Playing), (20, "LowerThird#2".into(), InstState::Playing)]);
    let c = s.state().unwrap();
    assert_eq!(c.layers[&10].current.as_ref().unwrap().props["homeScore"], 2, "данные дошли до табло");
    assert_eq!(c.layers[&20].current.as_ref().unwrap().props["name"], "Иван Петров");
    assert_eq!(c.data["homeScore"], 2);
    assert!(events(&s, "state").contains(&"LowerThird#1 removed".to_string()), "pregame-плашка снята после ухода");
    assert_eq!(events(&s, "state").iter().filter(|e| e.starts_with("channel ")).collect::<Vec<_>>(), ["channel pregame", "channel live", "channel interview"]);

    // Повтор того же состояния — ignored; break: табло остаётся, гость уходит.
    let acks = s.command(vec![state("interview")]).unwrap();
    assert_eq!(acks[0].status, AckStatus::Ok);
    assert!(events(&s, "ignored").iter().any(|e| e.contains("уже в этом состоянии")));
    s.advance(3.0).unwrap();
    s.command(vec![state("break")]).unwrap();
    let c = s.state().unwrap();
    assert_eq!(c.layers[&10].current.as_ref().unwrap().props["period"], "Перерыв");
    assert_eq!(c.layers[&10].current.as_ref().unwrap().props["clockRunning"], false);
    assert_eq!(layers(&s)[1].2, InstState::Leaving);
    s.frame_at(5.0, dev()).unwrap();
    assert_eq!(layers(&s), [(10, "Scoreboard#1".into(), InstState::Playing)], "гость снят после ухода");
}

#[test]
fn state_layers_refuse_direct_commands() {
    if home().is_none() {
        return;
    }
    let mut s = open(MATCH, &[]);
    s.command(vec![state("live")]).unwrap();
    let before = s.state().unwrap().clone();
    let acks = s
        .command(vec![
            cmd(json!({ "cmd": "play", "layer": 20, "component": "LowerThird", "props": { "name": "x", "role": "y", "live": true } })),
            cmd(json!({ "cmd": "stop", "layer": 10 })),
            cmd(json!({ "cmd": "update", "layer": 10, "props": { "homeScore": 3 } })),
            cmd(json!({ "cmd": "clear", "layer": 10 })),
            state("halftime"),
            data(json!({ "homeScore": -1 })),
            data(json!({ "score": 1 })),
        ])
        .unwrap();
    let errors: Vec<String> = acks.iter().map(|a| a.errors.join("; ")).collect();
    assert!(acks.iter().all(|a| a.status == AckStatus::Rejected), "{acks:?}");
    assert!(errors[0].contains("слой lower ведут состояния канала; переключай state <имя>"), "{errors:?}");
    assert!(errors[4].contains("такого состояния нет; есть: break, interview, live, pregame"), "{errors:?}");
    assert!(errors[5].contains("homeScore"), "{errors:?}");
    assert!(errors[6].contains("score"), "{errors:?}");
    assert_eq!(s.state().unwrap(), &before, "отказы ничего не меняют");
    // Команды компонента и next на слое состояний работают.
    s.advance(1.0).unwrap();
    let acks = s
        .command(vec![
            cmd(json!({ "cmd": "next", "layer": 10 })),
            cmd(json!({ "cmd": "invoke", "layer": 10, "command": "goal", "params": { "side": "home" } })),
        ])
        .unwrap();
    assert!(acks.iter().all(|a| a.status == AckStatus::Ok), "{acks:?}");
    // clear всего канала — аварийная кнопка: снимает и слои состояний.
    let acks = s.command(vec![cmd(json!({ "cmd": "clear" }))]).unwrap();
    assert_eq!(acks[0].status, AckStatus::Ok);
    assert!(layers(&s).is_empty());
    assert_eq!(s.state().unwrap().state, None);
}

#[test]
fn leaving_panel_turns_back() {
    if home().is_none() {
        return;
    }
    // Гость уходит (out), и посреди ухода оператор возвращает интервью: та же плашка
    // разворачивается с текущего места, второй копии нет.
    let mut s = open(MATCH, &[]);
    s.command(vec![data(json!({ "guest": "Иван Петров", "guestRole": "Тренер" })), state("interview")]).unwrap();
    s.advance(2.0).unwrap();
    s.command(vec![state("live")]).unwrap();
    s.advance(2.1).unwrap();
    let f = s.frame_at(2.1, dev()).unwrap();
    let panel = s.scene.find("LowerThird#1/panel").unwrap();
    let leaving = f.find(panel).unwrap().anim.as_ref().unwrap().motion.opacity;
    assert!(leaving < 1.0, "плашка уходит: {leaving}");
    s.command(vec![state("interview")]).unwrap();
    assert_eq!(layers(&s), [(10, "Scoreboard#1".into(), InstState::Playing), (20, "LowerThird#1".into(), InstState::Playing)]);
    let f = s.frame_at(2.12, dev()).unwrap();
    let back = f.find(panel).unwrap().anim.as_ref().unwrap().motion.opacity;
    assert!(back > leaving * 0.5 && back < 1.0, "in с текущего места, без скачка: {leaving} → {back}");
    s.frame_at(4.0, dev()).unwrap();
    assert_eq!(layers(&s).len(), 2, "развёрнутая плашка не снята после конца прежнего out");
    assert!(s.scene.find("LowerThird#2/panel").is_none(), "второй копии нет");
    assert!(!events(&s, "state").contains(&"LowerThird#1 removed".to_string()));
}

#[test]
fn snapshot_restores_data_and_state() {
    if home().is_none() {
        return;
    }
    let mut live = open(MATCH, &[]);
    live.command(vec![data(json!({ "homeScore": 3, "guest": "Анна", "guestRole": "Ведущая" })), state("interview")]).unwrap();
    live.frame_at(3.0, dev()).unwrap();
    let snapshot = live.state().unwrap().clone();
    let batch = snapshot.to_batch(0);
    let labels: Vec<String> = batch.cmds.iter().map(|e| e.cmd.label()).collect();
    assert_eq!(labels, ["update data", "state interview"], "слои состояний восстанавливает state, а не play");

    let mut restored = open(MATCH, &[batch]);
    restored.frame_at(0.0, dev()).unwrap();
    let r = restored.state().unwrap();
    assert_eq!(r.state, snapshot.state);
    assert_eq!(r.data, snapshot.data);
    for l in [10, 20] {
        let (a, b) = (r.layers[&l].current.as_ref().unwrap(), snapshot.layers[&l].current.as_ref().unwrap());
        assert_eq!((&a.component, &a.props), (&b.component, &b.props), "слой {l}");
    }
    // Сразу «в эфире»: в первом кадре плашка уже стоит.
    let f = restored.frame_at(0.0, dev()).unwrap();
    let panel = restored.scene.find("LowerThird#1/panel").unwrap();
    assert!(f.find(panel).unwrap().anim.as_ref().is_none_or(|a| a.motion.opacity == 1.0));
}

#[test]
fn match_show_replays_like_live() {
    if home().is_none() {
        return;
    }
    let journal = Journal::read(Path::new(SHOW)).unwrap();
    let mut live = open(MATCH, &[]);
    let mut seen = Vec::new();
    let samples = [0.5, 2.1, 3.4, 5.2, 6.3, 8.1, 8.3, 9.0, 10.4, 11.5];
    let mut batches = journal.batches.iter().peekable();
    for t in samples {
        while let Some(b) = batches.next_if(|b| live.frame_time(b.frame) <= t) {
            live.advance(live.frame_time(b.frame)).unwrap();
            let acks = live.command(b.cmds.clone()).unwrap();
            assert!(acks.iter().all(|a| a.status == AckStatus::Ok), "{acks:?}");
        }
        let f = live.frame_at(t, dev()).unwrap();
        let px = keyfill_render::render(&f, &RenderOptions::default()).unwrap().data_as_u8_slice().to_vec();
        seen.push((t, keyfill_core::dump::dump(&live.scene, &f), px));
    }
    assert_eq!(live.batches(), &journal.batches[..], "живая сессия пишет тот же журнал");
    let mut replay = open(MATCH, &journal.batches);
    for (t, dump, px) in &seen {
        let f = replay.frame_at(*t, dev()).unwrap();
        assert_eq!(&keyfill_core::dump::dump(&replay.scene, &f), dump, "дамп в момент {t}");
        let p = keyfill_render::render(&f, &RenderOptions::default()).unwrap();
        assert!(p.data_as_u8_slice() == &px[..], "пиксели в момент {t}");
    }
}

#[test]
fn state_that_cannot_be_built_fails_the_channel() {
    if home().is_none() {
        return;
    }
    let bundle = keyfill_cli::bundle(Path::new("crates/keyfill-cli/tests/fixtures/states/bad.tsx")).unwrap();
    let err = Session::open(&bundle, &RunOptions::default(), &[], &[]).err().expect("канал не открывается").to_string();
    assert!(err.contains("состояние on, слой main") && err.contains("text"), "{err}");
}
