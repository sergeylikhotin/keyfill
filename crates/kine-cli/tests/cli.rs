//! Контракт CLI: JSON-конверт `"kine": 1`, коды выхода, пропсы сцены, сегменты времени,
//! диагностики сборки и исключений JS. Тесты запускают сам бинарь, как это делает агент.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn home() -> Option<PathBuf> {
    let home = std::path::absolute(kine_cli::kine_home()).unwrap();
    if !home.join("node_modules/esbuild").exists() {
        eprintln!("skip: нет node_modules/esbuild, выполни npm install");
        return None;
    }
    Some(home)
}

/// Запуск `kine … --json` из корня репозитория: код выхода и конверт.
fn kine(args: &[&str]) -> Option<(i32, Value)> {
    let home = home()?;
    let out = Command::new(env!("CARGO_BIN_EXE_kine")).args(args).arg("--json").current_dir(&home).output().unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    let json: Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| panic!("не JSON ({e}): {stdout}"));
    assert_eq!(json["kine"], 1, "{json}");
    let code = out.status.code().unwrap();
    assert_eq!(json["exit"], code, "exit в конверте и код процесса совпадают: {json}");
    Some((code, json))
}

#[test]
fn dump_json_is_structured_tree() {
    let Some((code, j)) = kine(&["dump", "examples/scoreboard/scene.cues.ts", "--t", "goalHome+0.1"]) else { return };
    assert_eq!(code, 0);
    assert_eq!(j["command"], "dump");
    assert_eq!(j["ok"], true);
    assert_eq!(j["at"]["frame"], 115, "goalHome проходится в 2.2 с (invoke goal), +0.1 → 2.3: {}", j["at"]);
    let root = &j["result"]["nodes"][0];
    assert_eq!(root["path"], "Scoreboard#1/panel");
    assert_eq!(root["instance"], "Scoreboard#1");
    assert!(root["rect"].as_array().unwrap().iter().all(|v| v.is_number()));
    // Числа как в тексте: без шума f32.
    assert_eq!(j["result"]["stage"]["actionSafe"], serde_json::json!([67.2, 37.8, 1785.6, 1004.4]));
    assert!(j["repro"].as_str().unwrap().starts_with("kine dump examples/scoreboard/scene.cues.ts --t 2.3"), "{}", j["repro"]);
}

#[test]
fn dump_filters_by_instance_and_depth() {
    let Some((_, j)) = kine(&["dump", "examples/scoreboard/scene.tsx", "--path", "Scoreboard#1", "--depth", "1"]) else { return };
    let nodes = j["result"]["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 1);
    let grid = &nodes[0]["children"][0];
    assert!(grid.get("children").is_none(), "глубина 1: у внуков нет детей");
    assert_eq!(grid["cut"], 6);
}

#[test]
fn scene_props_from_set_and_errors() {
    let Some((code, j)) = kine(&["inspect", "examples/lower-third/scene.tsx", "LowerThird#1/title", "--set", "name=Иван Петров"]) else { return };
    assert_eq!(code, 0);
    assert_eq!(j["result"]["text"]["content"], "Иван Петров");

    let (code, j) = kine(&["dump", "examples/lower-third/scene.tsx", "--set", "nam=x"]).unwrap();
    assert_eq!(code, 2);
    assert_eq!(j["error"]["kind"], "usage");
    assert!(j["error"]["message"].as_str().unwrap().contains("нет пропса \"nam\"; есть: name, role"), "{j}");
    assert_eq!(j["next"][0], "kine schema examples/lower-third/scene.tsx");

    let (code, j) = kine(&["dump", "examples/lower-third/scene.tsx", "--set", "name="]).unwrap();
    assert_eq!(code, 0, "пустая строка — тоже строка: {j}");
}

#[test]
fn schema_lists_props_components_markers() {
    let Some((_, j)) = kine(&["schema", "examples/scoreboard/scene.cues.ts"]) else { return };
    let r = &j["result"];
    let sb = &r["components"][0];
    assert_eq!(sb["name"], "Scoreboard");
    assert_eq!(sb["instances"][0], "Scoreboard#1");
    assert_eq!(sb["commands"][0]["name"], "goal");
    assert_eq!(sb["commands"][0]["params"][0], serde_json::json!({ "name": "side", "type": "string" }));
    assert_eq!(sb["timeline"]["markers"][0], serde_json::json!(["ready", 0.7]));
    assert_eq!(sb["timeline"]["holds"][0], "ready");
    assert_eq!(r["cues"][0], "play@0");

    let (_, j) = kine(&["schema", "examples/lower-third/scene.tsx"]).unwrap();
    assert_eq!(j["result"]["props"][0], serde_json::json!({ "name": "name", "type": "string", "default": "Анна Смирнова" }));
}

#[test]
fn check_range_reports_first_and_last_frame() {
    let Some((code, j)) = kine(&["check", "examples/scoreboard/overflow.tsx"]) else { return };
    assert_eq!(code, 1);
    let issue = &j["result"]["issues"][0];
    assert_eq!(issue["rule"], "must.fit");
    assert_eq!(issue["node"], "Scoreboard#1/away");
    assert_eq!(issue["first"]["frame"], 0);
    assert_eq!(j["next"][0], "kine inspect examples/scoreboard/overflow.tsx Scoreboard#1/away --t 0");

    // Весь сценарий табло чистый; сегмент in плашки — от play до settled.
    let (code, j) = kine(&["check", "examples/scoreboard/scene.cues.ts"]).unwrap();
    assert_eq!(code, 0);
    assert_eq!(j["result"]["frames"], 231);
    let (_, j) = kine(&["check", "examples/lower-third/scene.cues.ts", "--t", "in"]).unwrap();
    let range = |j: &Value| (j["result"]["from"].as_f64().unwrap(), j["result"]["to"].as_f64().unwrap());
    assert_eq!(range(&j), (0.0, 0.46));
    let (_, j) = kine(&["check", "examples/scoreboard/scene.cues.ts", "--t", "loop"]).unwrap();
    assert_eq!(range(&j), (1.5, 4.2));

    let (code, j) = kine(&["check", "examples/scoreboard/scene.cues.ts", "--t", "nope"]).unwrap();
    assert_eq!(code, 2);
    assert!(j["error"]["message"].as_str().unwrap().ends_with("есть: in, out, ready, loop, goalHome, goalAway"), "{j}");
}

#[test]
fn js_exception_points_to_tsx() {
    let Some((code, j)) = kine(&["render", "crates/kine-cli/tests/fixtures/throws.tsx", "-o", "out/throws.png"]) else { return };
    assert_eq!(code, 4);
    assert_eq!(j["error"]["kind"], "js");
    let d = &j["error"]["diagnostics"][0];
    assert_eq!(d["rule"], "js.exception");
    assert_eq!(d["source"], "crates/kine-cli/tests/fixtures/throws.tsx:6:10", "{j}");
    let stack = d["detail"]["stack"].as_array().unwrap();
    assert!(stack.iter().all(|f| !f.as_str().unwrap().contains("eval_script")), "стек переведён в исходники: {stack:?}");
}

#[test]
fn build_error_is_diagnostic() {
    let Some((code, j)) = kine(&["dump", "crates/kine-cli/tests/fixtures/broken.tsx"]) else { return };
    assert_eq!(code, 3);
    let d = &j["error"]["diagnostics"][0];
    assert_eq!(d["rule"], "build.syntax");
    assert!(d["source"].as_str().unwrap().starts_with("crates/kine-cli/tests/fixtures/broken.tsx:7:"), "{d}");
    assert!(d["detail"]["line"].as_str().unwrap().contains("<Box />"));
}

#[test]
fn usage_errors_exit_2() {
    let Some((code, j)) = kine(&["render", "examples/lower-third/scene.tsx", "--t", "0..1"]) else { return };
    assert_eq!(code, 2);
    assert_eq!(j["error"]["kind"], "usage");
    let (code, _) = kine(&["frobnicate", "x"]).unwrap();
    assert_eq!(code, 2);
    let (code, _) = kine(&["dump", "examples/nope.tsx"]).unwrap();
    assert_eq!(code, 2);
}
