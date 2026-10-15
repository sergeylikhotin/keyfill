//! Текстовые представления кадра для агента: дамп сцены и inspect одного узла.

use std::fmt::Write as _;

use serde_json::Value;

use crate::diag::Level;
use crate::layout::{Frame, FrameNode, Rect};
use crate::look::{Look, num};
use crate::scene::{Kind, Scene};

fn rect(r: &Rect) -> String {
    format!("({},{} {}×{})", num(r.x), num(r.y), num(r.w), num(r.h))
}

fn plain_rect(r: &Rect) -> String {
    format!("{},{} {}×{}", num(r.x), num(r.y), num(r.w), num(r.h))
}

/// Токен, если значение пришло из темы, иначе само значение.
fn color_ref(look: &Look, key: &str, fallback: String) -> String {
    look.token(key).map_or(fallback, String::from)
}

pub fn dump(scene: &Scene, frame: &Frame) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "stage {}×{} @{}fps · actionSafe {} · titleSafe {}",
        num(frame.size[0]),
        num(frame.size[1]),
        num(frame.fps),
        plain_rect(&frame.action_safe),
        plain_rect(&frame.title_safe)
    );
    if let Some(stage) = &scene.stage {
        let children = &scene.node(stage.id).children;
        for (i, &c) in children.iter().enumerate() {
            node_lines(scene, frame, c, "", i + 1 == children.len(), &mut out);
        }
    }

    let diags = &frame.diagnostics.0;
    if !diags.is_empty() {
        let mut sorted: Vec<_> = diags.iter().collect();
        sorted.sort_by_key(|d| std::cmp::Reverse(d.level));
        let count = |l: Level| diags.iter().filter(|d| d.level == l).count();
        let _ = writeln!(out, "\ndiagnostics: {} error, {} warn, {} info", count(Level::Error), count(Level::Warn), count(Level::Info));
        for d in sorted {
            let _ = writeln!(out, "{d}");
        }
    }
    out
}

fn node_lines(scene: &Scene, frame: &Frame, id: u32, prefix: &str, last: bool, out: &mut String) {
    let node = scene.node(id);
    let fnode = frame.find(id);
    let branch = if last { "└─ " } else { "├─ " };
    let mut line = format!("{prefix}{branch}{}  {}", scene.path(id), node.kind.name());

    if let Some(kind) = node.props.get("layout").and_then(|l| l.get("kind")).and_then(Value::as_str) {
        let _ = write!(line, " {kind}");
        if let Some(gap) = node.props["layout"].get("gap").and_then(Value::as_f64) {
            let _ = write!(line, " gap {}", num(gap as f32));
        }
    }
    match fnode {
        Some(f) => {
            let _ = write!(line, "  {}", rect(&f.rect));
            summary(frame, f, &mut line);
        }
        None => line.push_str("  (hidden: when=false)"),
    }
    annotations(&node.props, &mut line);
    let _ = writeln!(out, "{line}");

    let child_prefix = format!("{prefix}{}", if last { "   " } else { "│  " });
    for (i, &c) in node.children.iter().enumerate() {
        node_lines(scene, frame, c, &child_prefix, i + 1 == node.children.len(), out);
    }
}

fn summary(frame: &Frame, f: &FrameNode, line: &mut String) {
    let v = &f.visual;
    let mut parts: Vec<String> = Vec::new();

    if let Some((index, _)) = f.text {
        let block = &frame.texts[index];
        let family = block.resolved_family.clone().unwrap_or_else(|| "?".into());
        let mut text = format!("{:?}  {family} {} {}px", block.content, num(block.look.weight), num(block.look.size));
        if let Some(requested) = &block.look.family
            && !requested.eq_ignore_ascii_case(&family) {
                let _ = write!(text, " (запрошен {requested})");
            }
        parts.push(text);
        parts.push(color_ref(&f.look, "color", block.look.color.to_string()));
        if block.line_count() > 1 {
            parts.push(format!("строк: {}", block.line_count()));
        }
    }
    if let Some(bg) = v.background {
        parts.push(format!("bg {}", color_ref(&f.look, "background", bg.to_string())));
    }
    if v.radius != [0.0; 4] {
        let r = if v.radius.iter().all(|r| *r == v.radius[0]) { num(v.radius[0]) } else { v.radius.map(num).join(",") };
        parts.push(format!("r {r}"));
    }
    if v.padding != [0.0; 4] {
        let [t, r, b, l] = v.padding;
        let p = if t == b && r == l { format!("{},{}", num(t), num(r)) } else { [t, r, b, l].map(num).join(",") };
        parts.push(format!("pad {p}"));
    }
    if let Some(b) = &v.border {
        let paint = f.look.props.get("border").and_then(|r| r.prov.tokens.first().cloned()).unwrap_or_else(|| b.color.to_string());
        let mut s = format!("border {} {paint}", num(b.width));
        if let Some(r) = f.look.props.get("border")
            && r.prov.layer.ends_with(" on") {
                let _ = write!(s, " ({})", r.prov.layer);
            }
        parts.push(s);
    }
    if !v.shadows.is_empty() {
        parts.push(format!("shadow×{}", v.shadows.len()));
    }
    if v.opacity < 1.0 {
        parts.push(format!("opacity {}", num(v.opacity)));
    }
    if v.transform.is_some() {
        parts.push("transform".into());
    }
    if !parts.is_empty() {
        let _ = write!(line, "  {}", parts.join(" · "));
    }
}

/// То, что объявлено, но в M0 ещё не исполняется: агент должен это видеть, а не гадать.
fn annotations(props: &std::collections::BTreeMap<String, Value>, line: &mut String) {
    if let Some(p) = props.get("presence").and_then(Value::as_object) {
        let keys: Vec<&str> = p.keys().map(String::as_str).collect();
        let _ = write!(line, "  [presence {} — с M2]", keys.join(","));
    }
    if let Some(m) = props.get("must").and_then(Value::as_object) {
        let keys: Vec<&str> = m.keys().map(String::as_str).collect();
        let _ = write!(line, "  [must {} — с M1]", keys.join(","));
    }
    if props.contains_key("else") {
        line.push_str("  [else — с M1]");
    }
    if props.contains_key("fit") {
        line.push_str("  [fit — с M1]");
    }
}

pub fn inspect(scene: &Scene, frame: &Frame, path: &str) -> Option<String> {
    let id = scene.find(path)?;
    let node = scene.node(id);
    let mut out = String::new();
    let _ = writeln!(out, "{}  {}", scene.path(id), node.kind.name());
    if let Some(inst) = node.instance() {
        let _ = writeln!(out, "instance  {inst}");
    }
    if let Some(src) = &node.src {
        let _ = writeln!(out, "source    {src}");
    }
    let Some(f) = frame.find(id) else {
        let _ = writeln!(out, "hidden    when=false, в кадре отсутствует");
        return Some(out);
    };
    let _ = writeln!(out, "rect      {}", plain_rect(&f.rect));

    if let Some((index, origin)) = f.text {
        let block = &frame.texts[index];
        let _ = writeln!(out, "content   {:?}", block.content);
        let _ = writeln!(
            out,
            "font      {} (запрошен {}), строк: {}, {}×{} at {},{}",
            block.resolved_family.as_deref().unwrap_or("?"),
            block.look.family.as_deref().unwrap_or("sans-serif"),
            block.line_count(),
            num(block.layout.width()),
            num(block.layout.height()),
            num(origin[0]),
            num(origin[1])
        );
    }

    if !f.look.props.is_empty() {
        let _ = writeln!(out, "look");
        for (key, r) in &f.look.props {
            let mut chain = Vec::new();
            for t in &r.prov.tokens {
                chain.push(format!("token {t}"));
            }
            chain.push(r.prov.layer.clone());
            let _ = writeln!(out, "  {key:<11} {:<40} ← {}", r.value.to_string(), chain.join(" ← "));
        }
    }

    let skip = ["look", "instance", "part", "hidden"];
    let rest: Vec<_> = node.props.iter().filter(|(k, _)| !skip.contains(&k.as_str())).collect();
    if !rest.is_empty() {
        let _ = writeln!(out, "props");
        for (k, v) in rest {
            let _ = writeln!(out, "  {k:<11} {v}");
        }
    }
    if node.kind != Kind::Text && !node.children.is_empty() {
        let names: Vec<String> = node.children.iter().map(|c| scene.path(*c)).collect();
        let _ = writeln!(out, "children  {}", names.join(", "));
    }
    let own: Vec<_> = frame.diagnostics.0.iter().filter(|d| d.node == scene.path(id)).collect();
    if !own.is_empty() {
        let _ = writeln!(out, "diagnostics");
        for d in own {
            let _ = writeln!(out, "  {d}");
        }
    }
    Some(out)
}
