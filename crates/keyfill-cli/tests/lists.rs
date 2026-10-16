//! Списки по данным (`For`): вход новых элементов, уход пропавших, обновление тех же по ключу,
//! стабильные адреса и порядок, удаление узлов после ухода.

use std::path::{Path, PathBuf};
use std::sync::Once;

use keyfill_cli::Session;
use keyfill_cli::channel::{AckStatus, Entry};
use keyfill_core::diag::Level;
use keyfill_core::{EvalOptions, Frame, Mode};
use keyfill_js::RunOptions;
use serde_json::json;

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

const TABLE: &str = "examples/standings/scene.cues.ts";
const ROWS: &str = "Standings#1/panel/box[0]/box";

fn open(entry: &str) -> Session {
    let bundle = keyfill_cli::bundle(Path::new(entry)).unwrap();
    Session::open(&bundle, &RunOptions::default(), &[], &[]).unwrap()
}

fn dev() -> EvalOptions {
    EvalOptions { mode: Mode::Dev, background: None }
}

/// Ключи строк таблицы в порядке дерева.
fn rows(s: &Session) -> Vec<String> {
    let id = s.scene.find("Standings#1/panel/box[0]").unwrap();
    s.scene.node(id).children.iter().filter_map(|c| s.scene.node(*c).key().map(String::from)).collect()
}

fn text(s: &Session, frame: &Frame, path: &str) -> String {
    let id = s.scene.find(path).unwrap_or_else(|| panic!("нет узла {path}"));
    let dump = keyfill_core::dump::dump(&s.scene, frame);
    let line = dump.lines().find(|l| l.contains(&format!("{} ", s.scene.path(id)))).unwrap_or_else(|| panic!("{path} нет в дампе"));
    line.split('"').nth(1).unwrap_or_default().to_string()
}

#[test]
fn rows_follow_data_by_key() {
    if home().is_none() {
        return;
    }
    let mut s = open(TABLE);
    s.frame_at(1.0, dev()).unwrap();
    assert_eq!(rows(&s), ["zen", "kra", "din"]);
    let zen = s.scene.find(&format!("{ROWS}[key=zen]")).unwrap();

    // update@1.5: Динамо уходит через out и остаётся на своём месте, Спартак входит вторым.
    let frame = s.frame_at(1.56, dev()).unwrap();
    assert_eq!(rows(&s), ["zen", "spa", "kra", "din"]);
    let dump = keyfill_core::dump::dump(&s.scene, &frame);
    let din = dump.lines().find(|l| l.contains("box[key=din]  ")).unwrap();
    assert!(din.contains("[out "), "{din}");
    let spa = dump.lines().find(|l| l.contains("box[key=spa]  ")).unwrap();
    assert!(spa.contains("motion opacity×"), "каскад stagger контейнера на новой строке: {spa}");
    assert_eq!(s.scene.find(&format!("{ROWS}[key=zen]")), Some(zen), "тот же ключ — тот же узел");

    // После ухода узлы Динамо удалены; место и очки — по новым данным, в тех же узлах.
    let frame = s.frame_at(2.5, dev()).unwrap();
    assert_eq!(rows(&s), ["zen", "spa", "kra"]);
    assert!(s.scene.find(&format!("{ROWS}[key=din]")).is_none());
    assert_eq!(text(&s, &frame, &format!("{ROWS}[key=zen]/text[2]")), "44");
    assert_eq!(text(&s, &frame, &format!("{ROWS}[key=spa]/text[0]")), "2");

    // update@3: Спартак первым — переставлен, Динамо снова входит последним (новые узлы).
    let frame = s.frame_at(3.5, dev()).unwrap();
    assert_eq!(rows(&s), ["spa", "zen", "kra", "din"]);
    assert_eq!(text(&s, &frame, &format!("{ROWS}[key=spa]/text[0]")), "1");
    assert_eq!(s.scene.find(&format!("{ROWS}[key=zen]")), Some(zen));
    assert!(frame.diagnostics.0.iter().all(|d| d.level != Level::Error), "{:?}", frame.diagnostics);
}

/// Кадр зависит только от момента: перемотка и прямой запрос дают один дамп.
#[test]
fn list_frames_do_not_depend_on_the_path() {
    if home().is_none() {
        return;
    }
    let mut walked = open(TABLE);
    for t in [0.5, 1.56, 2.0, 3.06] {
        walked.frame_at(t, dev()).unwrap();
    }
    let a = walked.frame_at(3.1, dev()).unwrap();
    let mut direct = open(TABLE);
    let b = direct.frame_at(3.1, dev()).unwrap();
    assert_eq!(keyfill_core::dump::dump(&walked.scene, &a), keyfill_core::dump::dump(&direct.scene, &b));
}

#[test]
fn duplicate_keys_warn_and_objects_need_by() {
    if home().is_none() {
        return;
    }
    let mut s = open("crates/keyfill-cli/tests/fixtures/lists/duplicate.tsx");
    let frame = s.frame_at(0.0, dev()).unwrap();
    let d = frame.diagnostics.0.iter().find(|d| d.rule == "list.duplicate-key").expect("list.duplicate-key");
    assert_eq!(d.level, Level::Warn);
    assert!(d.message.contains("ключ \"a\" повторяется"), "{d}");
    assert!(d.source.as_deref().unwrap_or("").contains("duplicate.tsx"), "{d}");
    let id = s.scene.find("stage").unwrap();
    let root = s.scene.node(id).children[0];
    assert_eq!(s.scene.node(root).children.len(), 2, "показаны a и b");

    let bundle = keyfill_cli::bundle(Path::new("crates/keyfill-cli/tests/fixtures/lists/no-key.tsx")).unwrap();
    let err = Session::open(&bundle, &RunOptions::default(), &[], &[]).err().unwrap();
    let msg = format!("{err:#}");
    assert!(msg.contains("у элементов-объектов нужен ключ: by={(item) => item.id}") && msg.contains("no-key.tsx"), "{msg}");
}

/// Список внутри экземпляра канала: строки уходят через out по данным, а при снятии
/// экземпляра уходят вместе с ним — узлов не остаётся.
#[test]
fn lists_inside_channel_instances() {
    if home().is_none() {
        return;
    }
    let cmd = |v: serde_json::Value| -> Entry { serde_json::from_value(v).unwrap() };
    let bundle = keyfill_cli::bundle(Path::new("crates/keyfill-cli/tests/fixtures/channel/channel.tsx")).unwrap();
    let mut s = Session::open(&bundle, &RunOptions::default(), &[], &[]).unwrap();
    let acks = s.command(vec![cmd(json!({ "cmd": "play", "layer": 1, "component": "Names", "props": { "names": ["a", "b", "c"] } }))]).unwrap();
    assert_eq!(acks[0].status, AckStatus::Ok, "{acks:?}");
    s.advance(0.4).unwrap();
    s.command(vec![cmd(json!({ "cmd": "update", "layer": 1, "props": { "names": ["a", "c", "d"] } }))]).unwrap();
    let frame = s.frame_at(0.48, dev()).unwrap();
    let dump = keyfill_core::dump::dump(&s.scene, &frame);
    assert!(dump.lines().any(|l| l.contains("text[key=b]") && l.contains("[out ")), "{dump}");
    s.frame_at(1.0, dev()).unwrap();
    assert!(s.scene.find("Names#1/text[key=b]").is_none(), "b удалён после out");
    assert!(s.scene.find("Names#1/text[key=d]").is_some());

    s.command(vec![cmd(json!({ "cmd": "stop", "layer": 1 }))]).unwrap();
    s.frame_at(2.0, dev()).unwrap();
    assert_eq!(s.scene.nodes.len(), 1, "остался только stage: {:?}", s.scene.nodes.keys().collect::<Vec<_>>());
    // Канал жив: следующий экземпляр ставится как обычно.
    let acks = s.command(vec![cmd(json!({ "cmd": "play", "layer": 1, "component": "Names", "props": { "names": ["x"] } }))]).unwrap();
    assert_eq!(acks[0].status, AckStatus::Ok, "{acks:?}");
    let frame = s.frame_at(2.5, dev()).unwrap();
    assert!(frame.diagnostics.0.iter().all(|d| d.level != Level::Error), "{:?}", frame.diagnostics);
}
