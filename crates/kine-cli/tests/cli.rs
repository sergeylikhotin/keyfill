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
    let (code, _) = kine(&["render", "examples/lower-third/scene.tsx", "-o", "out/no-such-dir/x.png"]).unwrap();
    assert_eq!(code, 2, "каталога для -o нет — ошибка вызова");
    let (code, _) = kine(&["render", "examples/lower-third/scene.tsx", "--scale", "0"]).unwrap();
    assert_eq!(code, 2);
}

#[test]
fn explain_and_docs_are_built_in() {
    let Some((code, j)) = kine(&["explain", "must.fit"]) else { return };
    assert_eq!(code, 0);
    assert_eq!(j["result"]["level"], "error");
    assert!(j["result"]["body"].as_str().unwrap().contains("shrinkTo"));
    let (code, j) = kine(&["explain", "must.fitt"]).unwrap();
    assert_eq!(code, 2);
    assert!(j["error"]["message"].as_str().unwrap().contains("похожие: must.fit"), "{j}");
    assert!(j.get("entry").is_none(), "имя правила — не сцена: {j}");
    let (_, j) = kine(&["docs"]).unwrap();
    let topics: Vec<&str> = j["result"].as_array().unwrap().iter().map(|t| t["topic"].as_str().unwrap()).collect();
    assert!(topics.contains(&"time") && topics.contains(&"test"), "{topics:?}");
    let (code, _) = kine(&["docs", "nope"]).unwrap();
    assert_eq!(code, 2);
}

#[test]
fn next_suggests_explain_for_first_error() {
    let Some((_, j)) = kine(&["check", "examples/scoreboard/overflow.tsx"]) else { return };
    assert_eq!(j["next"][1], "kine explain must.fit", "{}", j["next"]);
}

#[test]
fn config_names_scenes_and_output() {
    // kine.config.ts в корне: lower — сценарий плашки, out — каталог вывода.
    let Some((code, j)) = kine(&["render", "lower"]) else { return };
    assert_eq!(code, 0, "{j}");
    assert_eq!(j["entry"], "examples/lower-third/scene.cues.ts");
    assert_eq!(j["config"], "kine.config.ts");
    assert_eq!(j["result"]["file"], "out/examples/lower-third/scene.cues.png");
    let (code, j) = kine(&["dump", "lowr"]).unwrap();
    assert_eq!(code, 2);
    assert!(j["error"]["message"].as_str().unwrap().contains("именованные сцены: lower, score"), "{j}");
}

#[test]
fn examples_tests_pass_without_new_snapshots() {
    let Some((code, j)) = kine(&["test", "examples"]) else { return };
    assert_eq!(code, 0, "{j}");
    let counts = &j["result"]["counts"];
    assert_eq!(counts["failed"], 0);
    assert_eq!(counts["written"], 0, "снапшоты примеров лежат в репозитории");
    assert!(counts["passed"].as_u64().unwrap() >= 6);
}

#[test]
fn failing_tests_report_what_broke() {
    let Some((code, j)) = kine(&["test", "crates/kine-cli/tests/fixtures/suite/failing.suite.ts"]) else { return };
    assert_eq!(code, 1);
    let cases = j["result"]["cases"].as_array().unwrap();
    let case = |name: &str| cases.iter().find(|c| c["name"] == name).unwrap_or_else(|| panic!("нет случая {name}: {j}"));
    assert!(cases.iter().all(|c| c["ok"] == false), "{j}");

    let clean = &case("ошибка must")["moments"][0]["checks"][0];
    assert_eq!(clean["check"], "clean");
    assert_eq!(clean["issues"][0]["rule"], "must.fit");

    let dump = &case("снапшот разошёлся")["moments"][0]["checks"][1];
    assert_eq!(dump["status"], "changed");
    assert_eq!(dump["diff"][0], serde_json::json!({
        "op": "~",
        "node": "LowerThird#1/title",
        "old": "text  (154,902 437×68)  \"Иван Петров\"  Inter 600 56px · color.text  [must fit,contrast]  [else×3]",
        "new": "text  (154,902 437×68)  \"Анна Смирнова\"  Inter 600 56px · color.text  [must fit,contrast]  [else×3]",
    }));

    let moments = case("неверный момент")["moments"].as_array().unwrap();
    assert_eq!(moments[0]["error"]["kind"], "usage");
    assert_eq!(moments[1]["ok"], true, "ошибка одного момента не мешает остальным");

    assert_eq!(case("неверные пропсы")["moments"][0]["error"]["diagnostics"][0]["rule"], "props.invalid");
    let next: Vec<&str> = j["next"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(next, ["kine test crates/kine-cli/tests/fixtures/suite/failing.suite.ts --update", "kine explain must.fit"]);
}

#[test]
fn perf_reports_stages_against_budget() {
    let Some((code, j)) = kine(&["perf", "examples/lower-third/scene.tsx", "--t", "0..0.2"]) else { return };
    assert_eq!(code, 0, "perf — замер, а не проверка: {j}");
    let r = &j["result"];
    assert_eq!(r["frames"], 11);
    assert_eq!(r["budget_ms"], 20.0);
    let stages: Vec<&str> = r["stages"].as_array().unwrap().iter().map(|s| s["stage"].as_str().unwrap()).collect();
    assert_eq!(stages, ["script", "motion", "layout", "raster", "frame"]);
}

#[test]
fn bad_config_is_usage_error() {
    let Some(home) = home() else { return };
    let dir = home.join("out/config-test");
    std::fs::create_dir_all(&dir).unwrap();
    let core = kine_cli::slash(&home.join("packages/core/src/index.ts"));
    std::fs::write(dir.join("kine.config.ts"), format!("import {{ defineConfig }} from '{core}';\nexport default defineConfig({{ profiles: {{}} }} as never);\n")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_kine")).args(["schema", "../../examples/lower-third/scene.tsx", "--json"]).current_dir(&dir).output().unwrap();
    let j: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(out.status.code(), Some(2), "{j}");
    assert!(j["error"]["message"].as_str().unwrap().contains("profiles"), "{j}");
}

/// `kine … --json` в каталоге `dir` с переменными окружения.
fn kine_in(dir: &std::path::Path, args: &[&str], env: &[(&str, &std::path::Path)]) -> (i32, Value) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kine"));
    cmd.args(args).arg("--json").current_dir(dir);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    let json: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("не JSON ({e}): {}", String::from_utf8_lossy(&out.stdout)));
    (out.status.code().unwrap(), json)
}

#[test]
fn doctor_checks_the_pipeline() {
    let Some((code, j)) = kine(&["doctor"]) else { return };
    assert_eq!(code, 0, "{j}");
    let checks = j["result"]["checks"].as_array().unwrap();
    let status = |name: &str| checks.iter().find(|c| c["name"] == name).map(|c| c["status"].as_str().unwrap().to_string());
    for name in ["node", "@kine/core", "esbuild", "pipeline"] {
        assert_eq!(status(name).as_deref(), Some("ok"), "{name}: {j}");
    }
}

#[test]
fn cache_reports_and_cleans() {
    let Some(home) = home() else { return };
    let cache = std::env::temp_dir().join(format!("kine-cache-test-{}", std::process::id()));
    let env = [("KINE_CACHE_DIR", cache.as_path())];
    let (code, _) = kine_in(&home, &["schema", "lower"], &env);
    assert_eq!(code, 0);
    let (_, j) = kine_in(&home, &["cache"], &env);
    assert!(j["result"]["files"].as_u64().unwrap() >= 2, "сцена и конфиг: {j}");
    let (code, j) = kine_in(&home, &["cache", "clean"], &env);
    assert_eq!(code, 0);
    assert_eq!(j["result"]["cleaned"], true);
    assert!(!cache.exists());
    let (code, _) = kine_in(&home, &["cache", "purge"], &env);
    assert_eq!(code, 2);
}

#[test]
fn new_scene_passes_check_and_test() {
    let Some(_) = home() else { return };
    // Вне репозитория: там нет ни package.json, ни kine.config.ts — kine new создаёт проект.
    let dir = std::env::temp_dir().join(format!("kine-new-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (code, j) = kine_in(&dir, &["new", "lower-third"], &[]);
    assert_eq!(code, 0, "{j}");
    let files: Vec<&str> = j["result"]["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
    for f in ["lower-third/LowerThird.tsx", "lower-third/lower-third.kine.test.ts", "fonts/inter/Inter-Regular.otf", "tsconfig.json", "kine.config.ts"] {
        assert!(files.contains(&f), "{f}: {files:?}");
    }
    assert_eq!(j["next"][0], "kine check lower-third/scene.cues.ts");

    let (code, j) = kine_in(&dir, &["check", "lower-third"], &[]);
    assert_eq!(code, 0, "каталог сцены — точка входа: {j}");
    let (code, j) = kine_in(&dir, &["test"], &[]);
    assert_eq!(code, 0, "{j}");
    assert_eq!(j["result"]["counts"]["written"], 3);

    let (code, j) = kine_in(&dir, &["new", "title", "--template", "blank"], &[]);
    assert_eq!(code, 0, "{j}");
    assert!(j["result"]["kept"].as_array().unwrap().iter().any(|f| f == "kine.config.ts"), "проект уже есть: {j}");
    let (code, _) = kine_in(&dir, &["test", "title"], &[]);
    assert_eq!(code, 0);

    assert_eq!(kine_in(&dir, &["new", "lower-third"], &[]).0, 2, "каталог уже есть");
    assert_eq!(kine_in(&dir, &["new", "x", "--template", "nope"], &[]).0, 2);
    std::fs::remove_dir_all(&dir).ok();
}
