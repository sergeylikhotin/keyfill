//! `keyfill serve` целиком: настоящий процесс, WebSocket-клиент, HTTP. Команды в эфир и в
//! preview, take, события, `/state`, `/schema`, `/monitor`, токен, журнал и подъём из него.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tungstenite::{Message, WebSocket};

fn home() -> Option<PathBuf> {
    let home = std::path::absolute(keyfill_cli::keyfill_home()).unwrap();
    if !home.join("node_modules/esbuild").exists() {
        eprintln!("skip: нет node_modules/esbuild, выполни npm install");
        return None;
    }
    Some(home)
}

/// Живой сервер: процесс и адрес из первой строки stdout. Убивается при выходе из теста.
struct Serve {
    child: Child,
    addr: String,
}

impl Drop for Serve {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn serve(home: &Path, journal: &Path, extra: &[&str]) -> Serve {
    let mut child = Command::new(env!("CARGO_BIN_EXE_keyfill"))
        .args(["serve", "examples/channel/channel.tsx", "--port", "0", "--journal"])
        .arg(journal)
        .args(extra)
        .current_dir(home)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let v: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("первая строка serve ({e}): {line}"));
    Serve { child, addr: v["serve"].as_str().unwrap().to_string() }
}

fn http(addr: &str, path: &str) -> (u16, Vec<u8>) {
    let (status, _, body) = http_full(addr, path);
    (status, body)
}

/// Ответ HTTP целиком: код, заголовки (имена в нижнем регистре) и тело.
fn http_full(addr: &str, path: &str) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let mut s = TcpStream::connect(addr).unwrap();
    write!(s, "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).unwrap();
    let split = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&buf[..split]).to_string();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    let headers = head.lines().skip(1).filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))).collect();
    (status, headers, buf[split + 4..].to_vec())
}

fn json_get(addr: &str, path: &str) -> Value {
    let (status, body) = http(addr, path);
    assert_eq!(status, 200, "{path}: {}", String::from_utf8_lossy(&body));
    serde_json::from_slice(&body).unwrap()
}

struct Client {
    ws: WebSocket<TcpStream>,
    /// Всё, что пришло: события, состояние, preview, ответы.
    seen: Vec<Value>,
}

impl Client {
    fn connect(addr: &str, query: &str) -> Client {
        let stream = TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let (ws, _) = tungstenite::client::client(format!("ws://localhost/ws{query}"), stream).unwrap();
        let mut c = Client { ws, seen: Vec::new() };
        let hello = c.next();
        assert_eq!(hello["type"], "hello", "{hello}");
        c
    }

    fn next(&mut self) -> Value {
        loop {
            match self.ws.read().unwrap() {
                Message::Text(t) => {
                    let v: Value = serde_json::from_str(&t).unwrap();
                    self.seen.push(v.clone());
                    return v;
                }
                _ => continue,
            }
        }
    }

    /// Отправить и дождаться ответа с этим id.
    fn ask(&mut self, mut msg: Value, id: &str) -> Value {
        msg["id"] = json!(id);
        self.ws.send(Message::text(msg.to_string())).unwrap();
        let start = Instant::now();
        loop {
            let v = self.next();
            if v["id"] == id {
                return v;
            }
            assert!(start.elapsed() < Duration::from_secs(10), "нет ответа на {id}");
        }
    }

    /// Дождаться события.
    fn wait(&mut self, what: impl Fn(&Value) -> bool) -> Value {
        if let Some(v) = self.seen.iter().find(|v| what(v)) {
            return v.clone();
        }
        let start = Instant::now();
        loop {
            let v = self.next();
            if what(&v) {
                return v;
            }
            assert!(start.elapsed() < Duration::from_secs(10), "не дождался события");
        }
    }
}

fn lower(name: &str) -> Value {
    json!({ "cmd": "play", "layer": 20, "component": "LowerThird", "props": { "name": name, "role": "Гость", "live": false } })
}

#[test]
fn serve_takes_commands_preview_and_writes_journal() {
    let Some(home) = home() else { return };
    let journal = home.join("target/tmp-serve/journal");
    let _ = std::fs::remove_dir_all(&journal);
    let server = serve(&home, &journal, &[]);
    let addr = server.addr.clone();

    let root = json_get(&addr, "/");
    assert_eq!(root["channels"], json!([1]));
    let schema = json_get(&addr, "/schema");
    assert_eq!(schema["channels"]["1"]["catalog"], json!(["Scoreboard", "LowerThird"]), "{schema}");

    let mut c = Client::connect(&addr, "");
    // Команда в эфир: ответ после границы кадра, с номером кадра, затем события и состояние.
    let ack = c.ask(lower("Анна Смирнова"), "a1");
    assert_eq!(ack["type"], "ack", "{ack}");
    assert_eq!(ack["results"][0]["status"], "ok", "{ack}");
    assert!(ack["frame"].as_u64().is_some());
    c.wait(|v| v["type"] == "event" && v["source"] == "LowerThird#1" && v["name"] == "playing");

    // Отклонённая — с причиной, состояние не меняется.
    let ack = c.ask(json!({ "cmd": "update", "layer": 30, "props": { "name": "x" } }), "a2");
    assert_eq!(ack["results"][0]["status"], "rejected", "{ack}");
    let ack = c.ask(json!({ "cmd": "pause", "layer": 1 }), "a3");
    assert_eq!(ack["type"], "error", "{ack}");

    // Preview: замена плашки и данные — в отложенные; эфир не меняется, preview показывает итог.
    let ack = c.ask(json!({ "cmd": "batch", "to": "preview", "cmds": [lower("Иван Петров")] }), "p1");
    assert_eq!(ack["to"], "preview", "{ack}");
    assert_eq!(ack["results"][0]["status"], "ok", "{ack}");
    let ack = c.ask(json!({ "cmd": "update", "layer": 99, "props": {}, "to": "preview" }), "p2");
    assert_eq!(ack["results"][0]["status"], "rejected", "отклонённое не попадает в отложенные: {ack}");
    let state = json_get(&addr, "/state");
    let ch = &state["channels"]["1"];
    assert_eq!(ch["layers"]["20"]["current"]["props"]["name"], "Анна Смирнова", "{ch}");
    assert_eq!(ch["preview"]["staged"], json!(["play 20 LowerThird"]), "{ch}");
    assert!(ch["preview"]["t"].as_f64().is_some(), "{ch}");

    // Картинка эфира и preview.
    let (status, png) = http(&addr, "/monitor/1.png?scale=0.25");
    assert_eq!(status, 200);
    assert_eq!(&png[..4], b"\x89PNG");
    let (status, preview) = http(&addr, "/monitor/1.png?scale=0.25&preview=1");
    assert_eq!(status, 200);
    assert_ne!(png, preview, "preview показывает другую плашку");

    // take: отложенное — в эфир одной пачкой.
    let ack = c.ask(json!({ "cmd": "take" }), "t1");
    assert_eq!(ack["take"], true, "{ack}");
    assert_eq!(ack["results"][0]["status"], "ok", "{ack}");
    c.wait(|v| v["type"] == "event" && v["source"] == "LowerThird#2" && v["name"] == "playing");
    let state = json_get(&addr, "/state");
    assert_eq!(state["channels"]["1"]["preview"]["staged"], json!([]));
    assert_eq!(state["channels"]["1"]["layers"]["20"]["current"]["props"]["name"], "Иван Петров");

    // discard сбрасывает отложенное.
    c.ask(json!({ "cmd": "stop", "layer": 20, "to": "preview" }), "p3");
    let ack = c.ask(json!({ "cmd": "discard" }), "d1");
    assert_eq!(ack["discard"], true);
    assert_eq!(json_get(&addr, "/state")["channels"]["1"]["preview"]["staged"], json!([]));
    drop(c);
    drop(server);

    // Журнал записан по ходу эфира: принятые команды, без отклонённых, читается как <entry>.
    let file = std::fs::read_dir(&journal).unwrap().flatten().map(|e| e.path()).find(|p| p.to_string_lossy().ends_with(".journal.jsonl")).unwrap();
    let j = keyfill_cli::channel::Journal::read(&file).unwrap();
    let labels: Vec<String> = j.batches.iter().flat_map(|b| b.cmds.iter().map(|e| e.cmd.label())).collect();
    assert_eq!(labels, ["play 20 LowerThird", "play 20 LowerThird"], "{labels:?}");
    assert!(j.channel_path(&file).is_file(), "канал в заголовке — от каталога журнала: {}", j.header.channel);

    // Подъём: последний журнал → те же слои сразу «в эфир», без повторного in.
    let server = serve(&home, &journal, &["--resume"]);
    let state = json_get(&server.addr, "/state");
    let layer = &state["channels"]["1"]["layers"]["20"]["current"];
    assert_eq!(layer["props"]["name"], "Иван Петров", "{state}");
    let mut c = Client::connect(&server.addr, "");
    let st = c.ask(json!({ "cmd": "state" }), "s1");
    assert_eq!(st["layers"]["20"]["current"]["state"], "playing", "{st}");
}

#[test]
fn serve_needs_token_off_loopback_and_checks_it() {
    let Some(home) = home() else { return };
    let journal = home.join("target/tmp-serve/journal-token");
    // Из сети — только с токеном.
    let out = Command::new(env!("CARGO_BIN_EXE_keyfill"))
        .args(["serve", "examples/channel/channel.tsx", "--host", "0.0.0.0", "--port", "0", "--journal"])
        .arg(&journal)
        .current_dir(&home)
        .env_remove("KEYFILL_TOKEN")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("только с токеном"), "{}", String::from_utf8_lossy(&out.stderr));

    let server = serve(&home, &journal, &["--token", "s3cret"]);
    let (status, _) = http(&server.addr, "/state");
    assert_eq!(status, 401);
    let (status, _) = http(&server.addr, "/state?token=s3cret");
    assert_eq!(status, 200);
    let mut c = Client::connect(&server.addr, "?token=s3cret");
    let st = c.ask(json!({ "cmd": "state" }), "s1");
    assert_eq!(st["channel"], 1, "{st}");
}

/// Живой кадр эфира: номер кадра и пиксели.
fn capture(addr: &str) -> (u64, keyfill_render::Image) {
    let (status, headers, png) = http_full(addr, "/monitor/1.png?scale=1");
    assert_eq!(status, 200);
    let frame = headers.iter().find(|(k, _)| k == "x-keyfill-frame").map(|(_, v)| v.parse().unwrap()).expect("X-Keyfill-Frame");
    (frame, keyfill_render::decode_png(&png).unwrap())
}

/// Веха M4: скриптовая сессия через API даёт ожидаемые события, а журнал этой сессии,
/// воспроизведённый заново, даёт те же кадры, что видел эфир, — пиксель в пиксель.
#[test]
fn live_show_matches_its_journal() {
    let Some(home) = home() else { return };
    let journal = home.join("target/tmp-serve/journal-show");
    let _ = std::fs::remove_dir_all(&journal);
    let server = serve(&home, &journal, &[]);
    let addr = server.addr.clone();
    let mut c = Client::connect(&addr, "");
    let board = |c: &mut Client, name: &'static str| c.wait(move |v| v["type"] == "event" && v["source"] == "Scoreboard#1" && v["name"] == name);
    let mut shots = Vec::new();

    let ack = c.ask(
        json!({ "cmd": "play", "layer": 10, "component": "Scoreboard", "props": {
            "home": "Динамо Москва", "away": "Крылья Советов Самара", "homeScore": 2, "awayScore": 1,
            "period": "2-й тайм", "clockFrom": 4032, "live": true } }),
        "1",
    );
    assert_eq!(ack["results"][0]["status"], "ok", "{ack}");
    board(&mut c, "hold");
    shots.push(capture(&addr));
    c.ask(json!({ "cmd": "next", "layer": 10 }), "2");
    board(&mut c, "loop");
    shots.push(capture(&addr));
    c.ask(json!({ "cmd": "invoke", "layer": 10, "command": "goal", "params": { "side": "home" } }), "3");
    board(&mut c, "goal");
    shots.push(capture(&addr));
    // Плашка через preview: проверена до эфира, в эфир — take.
    let staged = c.ask(json!({ "cmd": "play", "layer": 20, "component": "LowerThird", "props": { "name": "Анна Смирнова", "role": "Ведущая", "live": true }, "to": "preview" }), "4");
    assert_eq!(staged["results"][0]["status"], "ok", "{staged}");
    let take = c.ask(json!({ "cmd": "take" }), "5");
    assert_eq!(take["results"][0]["status"], "ok", "{take}");
    c.wait(|v| v["type"] == "event" && v["source"] == "LowerThird#1" && v["name"] == "playing");
    std::thread::sleep(Duration::from_millis(150));
    shots.push(capture(&addr));
    c.ask(json!({ "cmd": "batch", "cmds": [{ "cmd": "stop", "layer": 20 }, { "cmd": "stop", "layer": 10 }] }), "6");
    std::thread::sleep(Duration::from_millis(120));
    shots.push(capture(&addr));
    board(&mut c, "removed");
    c.wait(|v| v["type"] == "event" && v["source"] == "LowerThird#1" && v["name"] == "removed");

    // События табло — в ожидаемом порядке (между ними могут быть и другие).
    let got: Vec<String> = c
        .seen
        .iter()
        .filter(|v| v["type"] == "event" && v["source"] == "Scoreboard#1")
        .map(|v| format!("{}:{}", v["kind"].as_str().unwrap(), v["name"].as_str().unwrap()))
        .collect();
    let want = ["state:loaded 10", "state:playing", "state:in", "marker:ready", "state:hold", "state:loop", "marker:goalHome", "emit:goal", "state:stopping", "state:removed"];
    let mut rest = got.iter();
    for w in want {
        assert!(rest.any(|g| g == w), "нет {w} по порядку в {got:?}");
    }
    drop(c);
    drop(server);

    // Воспроизведение журнала: те же кадры в те же моменты.
    let file = std::fs::read_dir(&journal).unwrap().flatten().map(|e| e.path()).find(|p| p.to_string_lossy().ends_with(".journal.jsonl")).unwrap();
    let j = keyfill_cli::channel::Journal::read(&file).unwrap();
    let bundle = keyfill_cli::bundle(&j.channel_path(&file)).unwrap();
    let run = keyfill_js::RunOptions { seed: j.header.seed, now_ms: j.header.now, ..Default::default() };
    let air = keyfill_core::EvalOptions { mode: keyfill_core::Mode::Air, background: None };
    assert_eq!(j.batches.len(), 5, "play, next, invoke, take, batch stop: {:?}", j.batches);
    for (n, live) in &shots {
        let mut replay = keyfill_cli::Session::open(&bundle, &run, &[], &j.batches).unwrap();
        let t = replay.frame_time(*n);
        let frame = replay.frame_at(t, air).unwrap();
        let pixmap = keyfill_render::render(&frame, &keyfill_render::RenderOptions::default()).unwrap();
        let img = keyfill_render::image(&pixmap);
        assert_eq!((img.width, img.height), (live.width, live.height));
        let diff = img.rgba.iter().zip(&live.rgba).filter(|(a, b)| a != b).count();
        assert_eq!(diff, 0, "кадр {n}: {diff} байт отличаются от эфира");
    }
    let frames: Vec<u64> = shots.iter().map(|(n, _)| *n).collect();
    assert!(frames.windows(2).all(|w| w[0] < w[1]), "снимки по ходу эфира: {frames:?}");
}
