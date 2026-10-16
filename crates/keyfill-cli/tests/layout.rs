//! Анимация геометрии (`transition.layout`): переезд при перестановке, после ухода соседа,
//! размер контейнера; кадр зависит только от момента; `must` смотрит на раскладку.

use std::path::{Path, PathBuf};
use std::sync::Once;

use keyfill_cli::Session;
use keyfill_cli::channel::{AckStatus, Entry};
use keyfill_core::diag::Level;
use keyfill_core::{EvalOptions, Frame, Mode, Rect};
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

const ROWS: &str = "crates/keyfill-cli/tests/fixtures/layout/rows.cues.ts";

fn open(entry: &str) -> Session {
    let bundle = keyfill_cli::bundle(Path::new(entry)).unwrap();
    Session::open(&bundle, &RunOptions::default(), &[], &[]).unwrap()
}

fn dev() -> EvalOptions {
    EvalOptions { mode: Mode::Dev, background: None }
}

/// Контейнер списка: единственный корень сцены.
fn list(s: &Session) -> u32 {
    let stage = s.scene.stage.as_ref().unwrap().id;
    s.scene.node(stage).children[0]
}

fn row(s: &Session, key: &str) -> u32 {
    let list = list(s);
    *s.scene.node(list).children.iter().find(|c| s.scene.node(**c).key() == Some(key)).unwrap_or_else(|| panic!("нет строки {key}"))
}

/// (видимое место, место в раскладке) узла.
fn rects(frame: &Frame, id: u32) -> (Rect, Rect) {
    let n = frame.find(id).unwrap();
    (n.shown, n.rect)
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

#[test]
fn rows_glide_to_new_places() {
    if home().is_none() {
        return;
    }
    let mut s = open(ROWS);
    let f = s.frame_at(0.5, dev()).unwrap();
    let (a, b, c) = (row(&s, "a"), row(&s, "b"), row(&s, "c"));
    assert_eq!([a, b, c].map(|id| rects(&f, id).0.y), [0.0, 40.0, 80.0]);

    // update@1: a и c меняются местами линейно за секунду, b стоит.
    let f = s.frame_at(1.5, dev()).unwrap();
    let (shown_a, rect_a) = rects(&f, a);
    assert!(near(shown_a.y, 40.0) && near(rect_a.y, 80.0), "a посреди переезда 0 → 80: {shown_a:?} {rect_a:?}");
    assert!(near(rects(&f, c).0.y, 40.0), "c посреди переезда 80 → 0");
    assert!(near(rects(&f, b).0.y, 40.0));
    let dump = keyfill_core::dump::dump(&s.scene, &f);
    let line = dump.lines().find(|l| l.contains("[key=a]  ")).unwrap();
    assert!(line.contains("(0,40 200×40 → 0,80 200×40)") && line.contains("layout 50%"), "{line}");

    let f = s.frame_at(2.1, dev()).unwrap();
    assert_eq!([a, b, c].map(|id| rects(&f, id).0.y), [80.0, 40.0, 0.0]);

    // update@3: b уходит на своём месте, a ждёт; после ухода (3.5) a поднимается, контейнер сжимается.
    let f = s.frame_at(3.25, dev()).unwrap();
    assert_eq!(rects(&f, a).0.y, 80.0);
    assert_eq!(rects(&f, b).0.y, 40.0);
    let f = s.frame_at(4.0, dev()).unwrap();
    assert!(!s.scene.nodes.contains_key(&b), "ушедшая строка удалена");
    assert!(near(rects(&f, a).0.y, 60.0), "a посреди переезда 80 → 40: {:?}", rects(&f, a));
    let (shown, rect) = rects(&f, list(&s));
    assert!(near(shown.h, 100.0) && near(rect.h, 80.0), "высота контейнера 120 → 80: {shown:?} {rect:?}");

    // update@5: две новые строки встают сразу, контейнер растёт; must.fit смотрит на раскладку,
    // а не на промежуточную высоту, поэтому строки «вне» растущего контейнера — не нарушение.
    let f = s.frame_at(5.5, dev()).unwrap();
    let (shown, rect) = rects(&f, list(&s));
    assert!(near(shown.h, 120.0) && near(rect.h, 160.0), "{shown:?} {rect:?}");
    assert_eq!(rects(&f, row(&s, "e")).0.y, 120.0);
    let errors: Vec<_> = f.diagnostics.0.iter().filter(|d| d.level == Level::Error).collect();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn interrupted_move_starts_where_the_row_is() {
    if home().is_none() {
        return;
    }
    // a едет 0 → 80 с 1 с; на 1.6 (y = 48) порядок вернули: a едет назад к 0 оттуда, где он
    // виден, за полную длительность перехода — без скачка.
    let mut s = open("crates/keyfill-cli/tests/fixtures/layout/back.cues.ts");
    let f = s.frame_at(1.56, dev()).unwrap();
    let a = row(&s, "a");
    assert!(near(rects(&f, a).0.y, 44.8), "{:?}", rects(&f, a));
    let f = s.frame_at(1.6, dev()).unwrap();
    assert!(near(rects(&f, a).0.y, 48.0), "в момент команды — там же: {:?}", rects(&f, a));
    let f = s.frame_at(2.1, dev()).unwrap();
    assert!(near(rects(&f, a).0.y, 24.0), "полпути 48 → 0: {:?}", rects(&f, a));
    let f = s.frame_at(2.6, dev()).unwrap();
    assert_eq!(rects(&f, a).0.y, 0.0);
}

#[test]
fn layout_motion_depends_only_on_the_moment() {
    if home().is_none() {
        return;
    }
    // Кадры подряд и сразу нужный момент дают одну и ту же картинку: раскладка снимается
    // в моменты команд и уходов, а не на отрисованных кадрах.
    let mut stepped = open(ROWS);
    let mut last = None;
    for n in 0..=110 {
        last = Some(stepped.frame_at(n as f64 / 25.0, dev()).unwrap());
    }
    let stepped_frame = last.unwrap();
    for t in [1.2, 3.8, 4.4] {
        let mut direct = open(ROWS);
        let f = direct.frame_at(t, dev()).unwrap();
        let mut again = open(ROWS);
        again.frame_at(t - 0.6, dev()).unwrap();
        let g = again.frame_at(t, dev()).unwrap();
        assert_eq!(boxes(&f), boxes(&g), "t={t}");
    }
    let mut direct = open(ROWS);
    let f = direct.frame_at(110.0 / 25.0, dev()).unwrap();
    assert_eq!(boxes(&f), boxes(&stepped_frame));
}

#[test]
fn live_channel_moves_like_its_journal() {
    if home().is_none() {
        return;
    }
    // Пружины, прерывание посреди переезда, уход соседа и две пачки на одном кадре: журнал
    // склеивает их в одну, а геометрия выходит та же.
    let entry = "crates/keyfill-cli/tests/fixtures/layout/channel.tsx";
    let update = |rows: &[&str]| cmd(json!({ "cmd": "update", "layer": 1, "props": { "rows": rows } }));
    let mut live = open(entry);
    let mut seen = Vec::new();
    let steps: [(f64, Vec<Entry>); 5] = [
        (0.0, vec![cmd(json!({ "cmd": "play", "layer": 1, "component": "Rows", "props": { "rows": ["a", "b", "c"] } }))]),
        (1.0, vec![update(&["c", "b", "a"])]),
        (1.2, vec![update(&["a", "c"])]),
        (1.2, vec![update(&["a", "c", "d"])]),
        (2.0, vec![update(&["d", "c"])]),
    ];
    // Кадры смотрят по ходу эфира, только вперёд: после последней пачки момента.
    let samples = [0.0, 0.4, 1.0, 1.08, 1.16, 1.2, 1.36, 1.6, 2.0, 2.12, 2.36, 2.8, 3.6];
    let mut steps = steps.into_iter().peekable();
    for t in samples {
        while let Some((at, cmds)) = steps.next_if(|(at, _)| *at <= t + 1e-9) {
            live.advance(at).unwrap();
            let acks = live.command(cmds).unwrap();
            assert!(acks.iter().all(|a| a.status == AckStatus::Ok), "{acks:?}");
        }
        let f = live.frame_at(t, dev()).unwrap();
        seen.push((t, boxes(&f), keyfill_core::dump::dump(&live.scene, &f)));
    }
    assert!(seen.iter().any(|(_, _, d)| d.contains("layout ")), "переезды были");
    let batches = live.batches().to_vec();
    assert_eq!(batches.len(), 4, "две пачки кадра 30 — одна");
    let bundle = keyfill_cli::bundle(Path::new(entry)).unwrap();
    let mut replay = Session::open(&bundle, &RunOptions::default(), &[], &batches).unwrap();
    for (t, b, d) in &seen {
        let f = replay.frame_at(*t, dev()).unwrap();
        assert_eq!(&keyfill_core::dump::dump(&replay.scene, &f), d, "дамп в момент {t}");
        assert_eq!(&boxes(&f), b, "рамки в момент {t}");
    }
}

fn cmd(v: serde_json::Value) -> Entry {
    serde_json::from_value(v).unwrap()
}

fn boxes(f: &Frame) -> Vec<(u32, [i32; 4])> {
    keyfill_core::display::node_boxes(f).iter().map(|b| (b.id, [b.rect.x, b.rect.y, b.rect.w, b.rect.h].map(|v| (v * 100.0).round() as i32))).collect()
}
