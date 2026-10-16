//! У каждого правила диагностики есть `kine explain`, и у каждого объяснения есть правило.
//! Правила ищутся в исходниках kine (`crates/*/src`, `packages/core/src`): строковые литералы
//! вида `"must.fit"` с известным пространством имён и шаги `fail("fit", ...)` в `must.rs`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const NAMESPACES: &[&str] =
    &["must", "anchor", "layout", "render", "look", "theme", "font", "text", "signal", "motion", "command", "timeline", "props", "js", "build", "perf"];

fn files(dir: &Path, ext: &[&str], out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            files(&p, ext, out);
        } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| ext.contains(&x)) {
            out.push(p);
        }
    }
}

/// Строковые литералы в кавычках `quote`, целиком похожие на имя правила.
fn literals(text: &str, quote: char, out: &mut BTreeSet<String>) {
    let mut parts = text.split(quote);
    parts.next();
    // Каждый второй кусок — внутри кавычек; экранированные кавычки в именах правил не встречаются.
    for (i, s) in parts.enumerate() {
        if i % 2 != 0 {
            continue;
        }
        let Some((ns, name)) = s.split_once('.') else { continue };
        let ok_name = !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        // `"theme.ts"` — имя файла, а не правило.
        let file = ["ts", "tsx", "rs", "mjs", "js", "json", "md", "txt", "otf", "png"].contains(&name);
        if NAMESPACES.contains(&ns) && ok_name && !file {
            out.insert(s.to_string());
        }
    }
}

fn rules_in_sources(home: &Path) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut paths = Vec::new();
    for e in std::fs::read_dir(home.join("crates")).unwrap().flatten() {
        files(&e.path().join("src"), &["rs"], &mut paths);
    }
    files(&home.join("packages/core/src"), &["ts"], &mut paths);
    for p in &paths {
        let text = std::fs::read_to_string(p).unwrap();
        // Литерал `"must.{}"` собирается из шага `fail("fit", ...)`.
        if p.ends_with("kine-core/src/must.rs") {
            for s in text.split("fail(\"").skip(1) {
                found.insert(format!("must.{}", s.split('"').next().unwrap()));
            }
        }
        for line in text.lines() {
            let code = line.trim_start();
            if code.starts_with("//") || code.starts_with("///") {
                continue;
            }
            literals(line, '"', &mut found);
            if p.extension().is_some_and(|x| x == "ts") {
                literals(line, '\'', &mut found);
            }
        }
    }
    found
}

#[test]
fn every_rule_has_explain() {
    let home = std::path::absolute(kine_cli::kine_home()).unwrap();
    let in_code = rules_in_sources(&home);
    let documented: BTreeSet<String> = kine_cli::help::rules().into_iter().map(|r| r.rule).collect();
    assert!(in_code.len() > 20, "поиск правил сломан: {in_code:?}");
    let missing: Vec<_> = in_code.difference(&documented).collect();
    assert!(missing.is_empty(), "нет раздела в help/rules.md: {missing:?}");
    let stale: Vec<_> = documented.difference(&in_code).collect();
    assert!(stale.is_empty(), "раздел без правила в исходниках: {stale:?}");
}
