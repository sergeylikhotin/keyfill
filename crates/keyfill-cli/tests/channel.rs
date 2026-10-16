//! Канал: команды слоям, журнал и его воспроизведение, снимок состояния.
//!
//! Главное свойство: живая сессия, записанная в журнал, при воспроизведении даёт те же кадры —
//! дерево, значения, события и пиксели.

use std::path::{Path, PathBuf};
use std::sync::Once;

use keyfill_cli::Session;
use keyfill_cli::channel::{AckStatus, Batch, Command, Entry, Header, InstState, Journal};
use keyfill_core::diag::Level;
use keyfill_core::{EvalOptions, Mode};
use keyfill_js::RunOptions;
use keyfill_render::RenderOptions;
use serde_json::{Map, Value, json};

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

const SHOW: &str = "examples/channel/channel.tsx";
const COUNTER: &str = "crates/keyfill-cli/tests/fixtures/channel/channel.tsx";

fn open(entry: &str, batches: &[Batch]) -> Session {
    let bundle = keyfill_cli::bundle(Path::new(entry)).unwrap();
    Session::open(&bundle, &RunOptions::default(), &[], batches).unwrap()
}

fn dev() -> EvalOptions {
    EvalOptions { mode: Mode::Dev, background: None }
}

fn props(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap()
}

fn cmd(v: Value) -> Entry {
    serde_json::from_value(v).unwrap()
}

/// Дамп и пиксели кадра: то, что должно совпасть у живой сессии и у журнала.
fn look(s: &mut Session, t: f64) -> (String, Vec<u8>) {
    let frame = s.frame_at(t, dev()).unwrap();
    let pixels = keyfill_render::render(&frame, &RenderOptions::default()).unwrap();
    (keyfill_core::dump::dump(&s.scene, &frame), pixels.data_as_u8_slice().to_vec())
}

fn scoreboard() -> Entry {
    cmd(json!({ "cmd": "play", "layer": 10, "component": "Scoreboard", "props": {
        "home": "Динамо Москва", "away": "Крылья Советов Самара", "homeScore": 2, "awayScore": 1,
        "period": "2-й тайм", "clockFrom": 4032, "live": true } }))
}

fn lower(name: &str, role: &str) -> Entry {
    cmd(json!({ "cmd": "play", "layer": 20, "component": "LowerThird", "props": { "name": name, "role": role, "live": true } }))
}

/// Живая сессия: команды приходят между кадрами, кадры смотрят по ходу. Её журнал,
/// воспроизведённый заново, даёт те же кадры в те же моменты.
#[test]
fn live_session_equals_its_journal() {
    if home().is_none() {
        return;
    }
    let mut live = open(SHOW, &[]);
    let mut seen = Vec::new();
    let mut at = |s: &mut Session, t: f64, cmds: Vec<Entry>| {
        s.advance(t).unwrap();
        let acks = s.command(cmds).unwrap();
        for t in [t, t + 0.3] {
            seen.push((t, look(s, t)));
        }
        acks
    };
    at(&mut live, 0.0, vec![scoreboard(), lower("Анна Смирнова", "Ведущая")]);
    at(&mut live, 1.2, vec![cmd(json!({ "cmd": "next", "layer": 10 }))]);
    let acks = at(&mut live, 2.2, vec![cmd(json!({ "cmd": "invoke", "layer": 10, "command": "goal", "params": { "side": "home" } }))]);
    assert_eq!(acks[0].status, AckStatus::Ok);
    // Отклонённая команда в журнал не попадает и ничего не меняет.
    let acks = at(&mut live, 3.0, vec![cmd(json!({ "cmd": "update", "layer": 10, "props": { "homeScore": -1 } })), lower("Иван Петров", "Тренер")]);
    assert_eq!(acks[0].status, AckStatus::Rejected, "{acks:?}");
    assert!(acks[0].errors[0].contains("homeScore: ожидается целое число ≥ 0, получено -1"), "{acks:?}");
    assert_eq!(acks[1].status, AckStatus::Ok);
    at(&mut live, 4.0, vec![cmd(json!({ "cmd": "update", "layer": 20, "props": { "role": "Главный тренер" } }))]);
    at(&mut live, 5.0, vec![cmd(json!({ "cmd": "stop", "layer": 20 })), cmd(json!({ "cmd": "stop", "layer": 10 }))]);

    let frames: Vec<u64> = live.batches().iter().map(|b| b.frame).collect();
    assert_eq!(frames, [0, 60, 110, 150, 200, 250]);
    assert_eq!(live.batches()[3].cmds.len(), 1, "отклонённый update не записан");

    // Журнал через файл: формат JSON Lines читается обратно без потерь.
    let journal = Journal { header: Header { keyfill: 1, channel: "channel.tsx".into(), seed: 1, now: 0.0 }, batches: live.batches().to_vec() };
    let journal = Journal::parse(&journal.to_jsonl()).unwrap();
    let mut replay = open(SHOW, &journal.batches);
    for (t, (dump, pixels)) in &seen {
        let (d, p) = look(&mut replay, *t);
        assert_eq!(&d, dump, "дамп в момент {t}");
        assert!(&p == pixels, "пиксели в момент {t}");
    }
    // Тот же журнал, открытый как файл, — его же кадры (по дампу финала).
    let mut again = open(SHOW, &journal.batches);
    assert_eq!(look(&mut again, 4.3).0, seen.iter().find(|(t, _)| (*t - 4.3).abs() < 1e-9).unwrap().1.0);
}

#[test]
fn rejected_commands_explain_and_change_nothing() {
    if home().is_none() {
        return;
    }
    let mut s = open(SHOW, &[]);
    let acks = s
        .command(vec![
            cmd(json!({ "cmd": "update", "layer": 20, "props": { "name": "x" } })),
            cmd(json!({ "cmd": "play", "layer": 20, "component": "Ticker" })),
            cmd(json!({ "cmd": "play", "layer": 20, "component": "LowerThird", "props": { "name": "x" } })),
            cmd(json!({ "cmd": "play", "layer": 20 })),
            cmd(json!({ "cmd": "next", "layer": 10 })),
        ])
        .unwrap();
    let errors: Vec<String> = acks.iter().map(|a| a.errors.join("; ")).collect();
    assert!(acks.iter().all(|a| a.status == AckStatus::Rejected), "{acks:?}");
    assert!(errors[0].contains("update 20: на слое никого нет"), "{errors:?}");
    assert!(errors[1].contains("нет компонента \"Ticker\"; есть: Scoreboard, LowerThird"), "{errors:?}");
    assert!(errors[2].contains("не задан обязательный пропс \"role\""), "{errors:?}");
    assert!(errors[3].contains("play 20: на слое ничего не загружено"), "{errors:?}");
    assert!(errors[4].contains("next 10: на слое ничего не показано"), "{errors:?}");
    assert!(s.batches().is_empty());
    assert!(s.state().unwrap().layers.is_empty());
    // Счётчик экземпляров не сдвинулся: первый настоящий будет LowerThird#1.
    s.command(vec![lower("Анна", "Ведущая")]).unwrap();
    assert_eq!(s.state().unwrap().layers[&20].current.as_ref().unwrap().name, "LowerThird#1");
}

#[test]
fn state_follows_layers_and_data() {
    if home().is_none() {
        return;
    }
    let mut s = open(SHOW, &[]);
    s.command(vec![scoreboard(), cmd(json!({ "cmd": "load", "layer": 30, "component": "LowerThird", "props": { "name": "Гость", "role": "Эксперт", "live": false } }))]).unwrap();
    s.advance(1.0).unwrap();
    s.command(vec![cmd(json!({ "cmd": "invoke", "layer": 10, "command": "goal", "params": { "side": "away" } }))]).unwrap();
    let state = s.state().unwrap().clone();
    let board = state.layers[&10].current.as_ref().unwrap();
    assert_eq!((board.state, board.props["awayScore"].clone()), (InstState::Playing, json!(2)), "ctx.set из команды виден в снимке");
    assert_eq!(board.props["clockRunning"], json!(true), "значение по умолчанию");
    let guest = state.layers[&30].loaded.as_ref().unwrap();
    assert_eq!(guest.state, InstState::Loaded);

    // Загруженный не виден, пока его не покажет play.
    let frame = s.frame_at(1.5, dev()).unwrap();
    let dump = keyfill_core::dump::dump(&s.scene, &frame);
    assert!(dump.contains("загружен на слой, ждёт play"), "{dump}");

    // Восстановление: команды снимка ставят на слои то же самое.
    let mut fresh = open(SHOW, &[state.to_batch(0)]);
    fresh.advance(0.0).unwrap();
    let restored = fresh.state().unwrap();
    assert_eq!(restored.layers[&10].current.as_ref().unwrap().props, board.props);
    assert!(restored.layers[&30].loaded.is_some());

    // stop: уходит через out и удаляется, слой пустеет.
    s.advance(2.0).unwrap();
    s.command(vec![cmd(json!({ "cmd": "stop", "layer": 10 })), cmd(json!({ "cmd": "clear", "layer": 30 }))]).unwrap();
    assert_eq!(s.state().unwrap().layers[&10].leaving.len(), 1);
    assert!(!s.state().unwrap().layers.contains_key(&30), "clear убирает сразу");
    s.advance(5.0).unwrap();
    assert!(s.state().unwrap().layers.is_empty(), "{:?}", s.state());
    assert!(s.scene.find("Scoreboard#1").is_none(), "узлы удалены");
    let removed = s.motion.events_log.iter().find(|e| e.source == "Scoreboard#1" && e.name == "removed").unwrap();
    assert!(removed.t > 2.0 && removed.t < 3.0, "removed в момент конца ухода: {}", removed.t);
}

/// Команда упала в JS посреди исполнения: частичный эффект остаётся, команда в журнале с
/// ошибкой, и воспроизведение повторяет её без расхождения.
#[test]
fn failed_command_is_journaled_and_replays() {
    if home().is_none() {
        return;
    }
    let mut s = open(COUNTER, &[]);
    s.command(vec![cmd(json!({ "cmd": "play", "layer": 1, "component": "Counter" }))]).unwrap();
    s.advance(0.4).unwrap();
    let acks = s.command(vec![cmd(json!({ "cmd": "invoke", "layer": 1, "command": "bump", "params": { "by": 5, "fail": true } }))]).unwrap();
    assert_eq!(acks[0].status, AckStatus::Failed, "{acks:?}");
    assert!(acks[0].errors[0].contains("bump упал"), "{acks:?}");
    assert_eq!(s.state().unwrap().layers[&1].current.as_ref().unwrap().props["n"], json!(5), "частичный эффект");
    let recorded = s.batches().last().unwrap().cmds[0].error.clone();
    assert!(recorded.is_some_and(|e| e.contains("bump упал")));
    let live = look(&mut s, 0.6).0;

    let mut replay = open(COUNTER, s.batches());
    let frame = replay.frame_at(0.6, dev()).unwrap();
    assert_eq!(keyfill_core::dump::dump(&replay.scene, &frame), live);
    assert!(!frame.diagnostics.0.iter().any(|d| d.rule == "journal.diverged"), "{:?}", frame.diagnostics);

    // Журнал говорит, что команда упала, а сейчас она проходит, — расхождение видно в диагностике.
    let mut batches = s.batches().to_vec();
    batches[1].cmds[0].cmd = Command::Invoke { layer: 1, command: "bump".into(), params: props(json!({ "by": 5 })) };
    let mut diverged = open(COUNTER, &batches);
    let frame = diverged.frame_at(0.6, dev()).unwrap();
    let d = frame.diagnostics.0.iter().find(|d| d.rule == "journal.diverged").expect("journal.diverged");
    assert_eq!(d.level, Level::Error);
    assert!(d.message.contains("кадр 10: invoke 1 bump: при записи команда упала"), "{d}");
}

/// Журнал, написанный руками с ошибкой: команда отклоняется с понятной причиной и местом.
#[test]
fn journal_rejects_are_diagnostics() {
    if home().is_none() {
        return;
    }
    let batches = vec![
        Batch { frame: 0, cmds: vec![cmd(json!({ "cmd": "play", "layer": 1, "component": "Counter", "props": { "n": 1.5 } }))] },
        Batch { frame: 5, cmds: vec![cmd(json!({ "cmd": "play", "layer": 1, "component": "Counter" }))] },
    ];
    let mut s = open(COUNTER, &batches);
    let frame = s.frame_at(0.4, dev()).unwrap();
    let d = frame.diagnostics.0.iter().find(|d| d.rule == "channel.rejected").expect("channel.rejected");
    assert!(d.message.contains("кадр 0: play 1 Counter: команда отклонена: n: ожидается целое число, получено 1.5"), "{d}");
    assert_eq!(s.state().unwrap().layers[&1].current.as_ref().unwrap().name, "Counter#1");
}

/// Типы пропсов в схеме: JSON Schema для пульта, подписи для текста.
#[test]
fn schema_describes_props_for_a_panel() {
    if home().is_none() {
        return;
    }
    let s = open(SHOW, &[]);
    let board = s.scene.components.iter().find(|c| c.name == "Scoreboard").unwrap();
    let score = board.props.iter().find(|p| p.name == "homeScore").unwrap();
    assert_eq!(score.ty, "integer 0..");
    assert_eq!(score.schema, json!({ "type": "integer", "minimum": 0, "default": 0 }));
    let side = &board.commands[0].params[0];
    assert_eq!(side.schema, json!({ "type": "string", "enum": ["home", "away"] }));
    assert!(side.required);
    let lower = s.scene.components.iter().find(|c| c.name == "LowerThird").unwrap();
    let tags = lower.props.iter().find(|p| p.name == "tags").unwrap();
    assert_eq!(tags.schema, json!({ "type": "array", "items": { "type": "string" }, "default": [] }));
    assert_eq!(s.scene.channel.as_ref().map(|c| c.components.as_slice()), Some(&["Scoreboard".to_string(), "LowerThird".to_string()][..]));
}
