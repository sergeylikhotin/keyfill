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

    // pregame → live (sequence(0.1)): плашка уходит, табло загружено и входит после неё.
    s.advance(1.0).unwrap();
    s.command(vec![state("live")]).unwrap();
    assert_eq!(layers(&s), [(10, "MatchCard#1".into(), InstState::Loaded), (20, "LowerThird#1".into(), InstState::Leaving)]);
    assert_eq!(s.state().unwrap().state.as_deref(), Some("live"));
    s.frame_at(1.4, dev()).unwrap();
    assert_eq!(layers(&s)[0], (10, "MatchCard#1".into(), InstState::Playing));

    // live → interview: табло — тот же экземпляр, гость входит снизу.
    s.advance(2.0).unwrap();
    let acks = s.command(vec![data(json!({ "guest": "Иван Петров", "guestRole": "Тренер", "homeScore": 2 })), state("interview")]).unwrap();
    assert!(acks.iter().all(|a| a.status == AckStatus::Ok), "{acks:?}");
    assert_eq!(layers(&s), [(10, "MatchCard#1".into(), InstState::Playing), (20, "LowerThird#2".into(), InstState::Loaded)]);
    s.frame_at(2.4, dev()).unwrap();
    assert_eq!(layers(&s)[1], (20, "LowerThird#2".into(), InstState::Playing), "гость вошёл через 0.3 с");
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
    assert_eq!(layers(&s), [(10, "MatchCard#1".into(), InstState::Playing)], "гость снят после ухода");
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
    assert!(errors[4].contains("такого состояния нет; есть: break, interview, intro, live, pregame, var"), "{errors:?}");
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
    s.command(vec![state("break")]).unwrap();
    s.advance(2.1).unwrap();
    let f = s.frame_at(2.1, dev()).unwrap();
    let panel = s.scene.find("LowerThird#1/panel").unwrap();
    let leaving = f.find(panel).unwrap().anim.as_ref().unwrap().motion.opacity;
    assert!(leaving < 1.0, "плашка уходит: {leaving}");
    s.command(vec![state("interview")]).unwrap();
    assert_eq!(layers(&s), [(10, "MatchCard#1".into(), InstState::Playing), (20, "LowerThird#1".into(), InstState::Playing)]);
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

/// (момент, источник, событие) жизненного цикла.
fn timeline(s: &Session) -> Vec<(f64, String, String)> {
    s.motion.events_log.iter().filter(|e| e.kind == "state").map(|e| ((e.t * 1000.0).round() / 1000.0, e.source.clone(), e.name.clone())).collect()
}

#[test]
fn transition_follows_its_plan() {
    if home().is_none() {
        return;
    }
    // pregame → live — sequence(0.1): табло входит через 0.1 с после конца ухода представления.
    let mut s = open(MATCH, &[]);
    s.command(vec![state("pregame")]).unwrap();
    s.advance(1.0).unwrap();
    s.command(vec![state("live")]).unwrap();
    let f = s.frame_at(1.1, dev()).unwrap();
    let plan = f.clock.as_ref().unwrap().transition.clone().expect("расписание в кадре");
    assert_eq!(plan.rule, "sequence +0.1 (pregame -> live)");
    let lines = plan.lines().join("\n");
    assert!(lines.contains("lower  out     0–0.25") && lines.contains("score  in      0.35–") && lines.contains("(после всех out +0.1)"), "{lines}");
    s.frame_at(2.0, dev()).unwrap();
    let ev = timeline(&s);
    assert!(ev.contains(&(1.0, "LowerThird#1".into(), "stopping".into())), "{ev:?}");
    assert!(ev.contains(&(1.35, "MatchCard#1".into(), "playing".into())), "табло вошло по расписанию: {ev:?}");
    let f = s.frame_at(4.0, dev()).unwrap();
    assert!(!keyfill_core::dump::dump(&s.scene, &f).contains("переход pregame"), "после перехода расписания в дампе нет");
}

#[test]
fn interrupted_transition_cancels_steps_not_started() {
    if home().is_none() {
        return;
    }
    // live пришёл, табло ждёт своего шага (1.35), а оператор вернул pregame в 1.1: табло так и не
    // показали, представление разворачивается с текущего места.
    let mut s = open(MATCH, &[]);
    s.command(vec![state("pregame")]).unwrap();
    s.advance(1.0).unwrap();
    s.command(vec![state("live")]).unwrap();
    s.advance(1.1).unwrap();
    s.command(vec![state("pregame")]).unwrap();
    s.frame_at(2.0, dev()).unwrap();
    assert_eq!(layers(&s), [(20, "LowerThird#1".into(), InstState::Playing)]);
    let ev = timeline(&s);
    assert!(ev.contains(&(1.1, "MatchCard#1".into(), "removed".into())), "{ev:?}");
    assert!(!ev.iter().any(|(_, src, name)| src == "MatchCard#1" && name == "playing"), "табло не показывали: {ev:?}");
    assert!(ev.contains(&(1.1, "LowerThird#1".into(), "playing".into())), "{ev:?}");

    // Разворот тоже ждёт своего шага: в live → interview гость входит через 0.3 с, а его прежний
    // out уже кончился — та же плашка входит заново, её не сняли.
    let mut s = open(MATCH, &[]);
    s.command(vec![data(json!({ "guest": "Иван Петров", "guestRole": "Тренер" })), state("interview")]).unwrap();
    s.advance(2.0).unwrap();
    s.command(vec![state("live")]).unwrap();
    s.advance(2.1).unwrap();
    s.command(vec![state("interview")]).unwrap();
    s.frame_at(3.0, dev()).unwrap();
    assert_eq!(layers(&s), [(10, "MatchCard#1".into(), InstState::Playing), (20, "LowerThird#1".into(), InstState::Playing)]);
    let ev = timeline(&s);
    assert!(ev.contains(&(2.4, "LowerThird#1".into(), "playing".into())), "{ev:?}");
    assert!(!ev.iter().any(|(_, src, name)| src == "LowerThird#1" && name == "removed"), "{ev:?}");
}

#[test]
fn overlapping_panels_warn_and_sequence_fixes_it() {
    if home().is_none() {
        return;
    }
    let mut s = open("crates/keyfill-cli/tests/fixtures/states/overlap.tsx", &[]);
    s.command(vec![state("one")]).unwrap();
    s.advance(1.0).unwrap();
    s.command(vec![state("two")]).unwrap();
    s.advance(2.0).unwrap();
    s.command(vec![state("one")]).unwrap();
    let f = s.frame_at(3.0, dev()).unwrap();
    let warns: Vec<_> = f.diagnostics.0.iter().filter(|d| d.rule == "transition.overlap").collect();
    assert_eq!(warns.len(), 1, "one → two наложение, two → one по очереди: {warns:?}");
    assert!(warns[0].message.contains("b (B#1) входит в 0 с, пока a (A#1) уходит до 0.4 с"), "{}", warns[0].message);
    assert!(warns[0].hint.as_deref().unwrap().contains("enter('b', { after: 'a:out' })"));
}

#[test]
fn transition_rules_are_checked_when_the_channel_loads() {
    if home().is_none() {
        return;
    }
    let err = |file: &str| {
        let bundle = keyfill_cli::bundle(Path::new(file)).unwrap();
        Session::open(&bundle, &RunOptions::default(), &[], &[]).err().expect("канал не открывается").to_string()
    };
    let e = err("crates/keyfill-cli/tests/fixtures/states/cycle.tsx");
    assert!(e.contains("переход one -> two: цикл — a:out ждёт b:in, b:in ждёт a:out"), "{e}");
    let e = err("crates/keyfill-cli/tests/fixtures/states/no-step.tsx");
    assert!(e.contains("шага a:in в этом переходе нет — на слое a здесь только out"), "{e}");
}

const MORPH: &str = "crates/keyfill-cli/tests/fixtures/states/morph.tsx";

/// Видимое место узла по адресу.
fn shown(s: &Session, f: &keyfill_core::Frame, path: &str) -> [f32; 4] {
    let id = s.scene.find(path).unwrap_or_else(|| panic!("нет {path}"));
    let r = f.find(id).unwrap_or_else(|| panic!("{path} нет в кадре")).shown;
    [r.x, r.y, r.w, r.h].map(|v| (v * 100.0).round() / 100.0)
}

#[test]
fn same_instance_changes_shape_and_place() {
    if home().is_none() {
        return;
    }
    // small → big: тот же экземпляр Card (нового нет), облик — пропс big, место — якорь
    // состояния; подложка едет из левого верхнего угла в правый нижний и растёт, линейно за секунду.
    let mut s = open(MORPH, &[]);
    s.command(vec![state("small")]).unwrap();
    s.advance(1.0).unwrap();
    s.command(vec![state("big")]).unwrap();
    assert_eq!(layers(&s), [(30, "Card#1".into(), InstState::Playing)], "тот же экземпляр");
    let f = s.frame_at(1.0, dev()).unwrap();
    assert_eq!(shown(&s, &f, "Card#1"), [0.0, 0.0, 100.0, 50.0], "в момент команды — где был");
    let f = s.frame_at(1.5, dev()).unwrap();
    assert_eq!(shown(&s, &f, "Card#1"), [170.0, 105.0, 200.0, 100.0], "полпути");
    let f = s.frame_at(2.1, dev()).unwrap();
    assert_eq!(shown(&s, &f, "Card#1"), [340.0, 210.0, 300.0, 150.0]);
    // Назад: состояние без якоря — снова свой якорь из view.
    s.command(vec![state("small")]).unwrap();
    let f = s.frame_at(3.2, dev()).unwrap();
    assert_eq!(shown(&s, &f, "Card#1"), [0.0, 0.0, 100.0, 50.0]);
    assert_eq!(layers(&s), [(30, "Card#1".into(), InstState::Playing)]);
}

#[test]
fn anchor_follows_a_part_on_another_layer() {
    if home().is_none() {
        return;
    }
    // Плашка стоит под подложкой слоя main и едет за ней, пока та растёт (transition.layout).
    let mut s = open(MORPH, &[]);
    s.command(vec![state("rows")]).unwrap();
    let f = s.frame_at(0.5, dev()).unwrap();
    assert_eq!(shown(&s, &f, "Tag#1"), [0.0, 50.0, 80.0, 20.0], "под подложкой высотой 40 + 10");
    s.advance(1.0).unwrap();
    s.command(vec![data(json!({ "rows": ["a", "b"] }))]).unwrap();
    let f = s.frame_at(1.5, dev()).unwrap();
    assert_eq!(shown(&s, &f, "Panel#1"), [0.0, 0.0, 200.0, 60.0]);
    assert_eq!(shown(&s, &f, "Tag#1"), [0.0, 70.0, 80.0, 20.0], "за видимой подложкой, а не за её конечной раскладкой");
    let tag = f.find(s.scene.find("Tag#1").unwrap()).unwrap();
    assert_eq!(tag.rect.y, 90.0, "место в раскладке — под конечной подложкой");
    let f = s.frame_at(2.1, dev()).unwrap();
    assert_eq!(shown(&s, &f, "Tag#1"), [0.0, 90.0, 80.0, 20.0]);
}
