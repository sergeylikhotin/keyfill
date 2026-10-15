//! Разрешение `look`: слои применяются по порядку, побеждает последний; токены темы
//! подставляются в ядре. Для каждого итогового свойства запоминается, откуда оно пришло.

use std::collections::BTreeMap;
use std::fmt;

use serde_json::Value;

use crate::diag::{Diagnostics, Level};
use crate::scene::{Kind, Node, Scene};

const BOX_KEYS: &[&str] = &[
    "background", "padding", "radius", "border", "shadow", "opacity", "blend", "clip", "transform", "transition",
];
const TEXT_KEYS: &[&str] = &["font", "size", "weight", "color", "lineHeight", "tracking", "align", "case"];

/// Откуда взялось значение: слой `look` и токены темы внутри него.
#[derive(Debug, Clone, PartialEq)]
pub struct Prov {
    pub layer: String,
    pub tokens: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Resolved {
    pub value: Value,
    pub prov: Prov,
}

#[derive(Debug, Clone, Default)]
pub struct Look {
    pub props: BTreeMap<String, Resolved>,
}

impl Look {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.props.get(key).map(|r| &r.value)
    }

    /// Короткое имя источника для дампа: токен, если он есть, иначе значение.
    pub fn token(&self, key: &str) -> Option<&str> {
        self.props.get(key)?.prov.tokens.first().map(String::as_str)
    }
}

/// Что стилизуется: от этого зависит набор допустимых свойств.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Box,
    Text,
    Span,
}

const SPAN_KEYS: &[&str] = &["font", "size", "weight", "color", "tracking", "case"];

/// Слои `look` узла плюс дополнительные (из шагов `else`), по порядку.
pub fn resolve(scene: &Scene, node: &Node, extra: &[Value], diags: &mut Diagnostics) -> Look {
    let own = node.props.get("look").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
    let target = if node.kind == Kind::Text { Target::Text } else { Target::Box };
    let path = scene.path(node.id);
    resolve_layers(scene, own.iter().chain(extra), target, node.kind.name(), &path, node.src.as_deref(), diags)
}

pub fn resolve_layers<'a>(
    scene: &Scene,
    layers: impl Iterator<Item = &'a Value>,
    target: Target,
    kind: &str,
    path: &str,
    src: Option<&str>,
    diags: &mut Diagnostics,
) -> Look {
    let mut look = Look::default();
    for layer in layers {
        let label = layer.get("label").and_then(Value::as_str).unwrap_or("look").to_string();
        let Some(props) = layer.get("props").and_then(Value::as_object) else { continue };
        for (key, raw) in props {
            let known_box = BOX_KEYS.contains(&key.as_str());
            let known_text = TEXT_KEYS.contains(&key.as_str());
            match target {
                Target::Box if known_text => {
                    diags.push(
                        Level::Warn,
                        "look.text-on-box",
                        path,
                        src,
                        format!("свойство {key} в {label} не действует на {kind}"),
                        Some("наследования нет: текстовые свойства задаются на самом <Text>"),
                    );
                    continue;
                }
                Target::Span if !SPAN_KEYS.contains(&key.as_str()) => {
                    diags.push(
                        Level::Warn,
                        "look.span-unsupported",
                        path,
                        src,
                        format!("свойство {key} в {label} не действует на Span"),
                        Some("у Span есть только font, size, weight, color, tracking, case"),
                    );
                    continue;
                }
                _ => {}
            }
            if !known_box && !known_text {
                diags.push(Level::Warn, "look.unknown", path, src, format!("неизвестное свойство {key} в {label}"), None);
                continue;
            }
            let mut tokens = Vec::new();
            let mut missing = Vec::new();
            let value = substitute(raw, &scene.theme, &mut tokens, &mut missing);
            for m in missing {
                diags.push(Level::Error, "theme.missing-token", path, src, format!("токен {m} не найден в теме"), None);
            }
            look.props.insert(key.clone(), Resolved { value, prov: Prov { layer: label.clone(), tokens } });
        }
    }
    look
}

fn substitute(v: &Value, theme: &BTreeMap<String, Value>, tokens: &mut Vec<String>, missing: &mut Vec<String>) -> Value {
    match v {
        Value::Object(map) => {
            if let Some(path) = map.get("$t").and_then(Value::as_str) {
                tokens.push(path.to_string());
                return match theme.get(path) {
                    Some(value) => value.clone(),
                    None => {
                        missing.push(path.to_string());
                        Value::Null
                    }
                };
            }
            Value::Object(map.iter().map(|(k, v)| (k.clone(), substitute(v, theme, tokens, missing))).collect())
        }
        Value::Array(items) => Value::Array(items.iter().map(|v| substitute(v, theme, tokens, missing)).collect()),
        other => other.clone(),
    }
}

// ─── Типизированные значения ────────────────────────────────────────────────

/// Цвет в sRGB, каналы 0..1, альфа не умножена.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const WHITE: Rgba = Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

    pub fn to_u8(self) -> [u8; 4] {
        let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        [c(self.r), c(self.g), c(self.b), c(self.a)]
    }
}

impl fmt::Display for Rgba {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [r, g, b, _] = self.to_u8();
        write!(f, "#{r:02x}{g:02x}{b:02x}")?;
        if self.a < 1.0 {
            write!(f, " a{}", num(self.a))?;
        }
        Ok(())
    }
}

/// Число без лишних нулей: 16, 0.92, 523.5.
pub fn num(v: f32) -> String {
    let rounded = (v * 100.0).round() / 100.0;
    if rounded.fract() == 0.0 { format!("{}", rounded as i64) } else { format!("{rounded}") }
}

pub fn parse_color(v: &Value) -> Result<Rgba, String> {
    let obj = v.as_object().ok_or_else(|| format!("цвет должен быть объектом, получено {v}"))?;
    let a = obj.get("a").and_then(Value::as_f64).unwrap_or(1.0) as f32;
    if let Some(lch) = obj.get("oklch").and_then(Value::as_array) {
        let f = |i: usize| lch.get(i).and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let [r, g, b] = oklch_to_srgb(f(0), f(1), f(2));
        return Ok(Rgba { r, g, b, a });
    }
    let ch = |k: &str| obj.get(k).and_then(Value::as_f64).map(|v| v as f32 / 255.0);
    match (ch("r"), ch("g"), ch("b")) {
        (Some(r), Some(g), Some(b)) => Ok(Rgba { r, g, b, a }),
        _ => Err(format!("цвет без r/g/b или oklch: {v}")),
    }
}

fn oklch_to_srgb(l: f32, c: f32, h: f32) -> [f32; 3] {
    let (a, b) = (c * h.to_radians().cos(), c * h.to_radians().sin());
    let l_ = (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m_ = (l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s_ = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    let lin = [
        4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_94 * s_,
        -1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_38 * s_,
        -0.004_196_086 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_,
    ];
    lin.map(|x| {
        let x = x.clamp(0.0, 1.0);
        if x <= 0.003_130_8 { 12.92 * x } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 }
    })
}

/// В M0 краска — только сплошной цвет; у градиента берётся первый стоп.
pub fn parse_paint(v: &Value) -> Result<(Rgba, bool), String> {
    for kind in ["linear", "radial"] {
        if let Some(g) = v.get(kind) {
            let first = g
                .get("stops")
                .and_then(Value::as_array)
                .and_then(|s| s.first())
                .and_then(|s| s.get(1))
                .ok_or("градиент без стопов")?;
            return Ok((parse_color(first)?, true));
        }
    }
    Ok((parse_color(v)?, false))
}

pub fn parse_len(v: &Value) -> Result<f32, String> {
    v.as_f64().map(|v| v as f32).ok_or_else(|| format!("ожидается число, получено {v}"))
}

/// `[top, right, bottom, left]`.
pub fn parse_edges(v: &Value) -> Result<[f32; 4], String> {
    if let Some(n) = v.as_f64() {
        return Ok([n as f32; 4]);
    }
    let items: Vec<f32> = v
        .as_array()
        .ok_or_else(|| format!("ожидается число или массив, получено {v}"))?
        .iter()
        .map(parse_len)
        .collect::<Result<_, _>>()?;
    match items[..] {
        [y, x] => Ok([y, x, y, x]),
        [t, r, b, l] => Ok([t, r, b, l]),
        _ => Err(format!("ожидается [y, x] или [top, right, bottom, left], получено {v}")),
    }
}

/// `[tl, tr, br, bl]`.
pub fn parse_corners(v: &Value) -> Result<[f32; 4], String> {
    if let Some(n) = v.as_f64() {
        return Ok([n as f32; 4]);
    }
    let items: Vec<f32> = v.as_array().ok_or("ожидается число или [tl, tr, br, bl]")?.iter().map(parse_len).collect::<Result<_, _>>()?;
    items.try_into().map_err(|_| "ожидается [tl, tr, br, bl]".to_string())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Shadow {
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: Rgba,
    pub inset: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Border {
    pub width: f32,
    pub color: Rgba,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Transform {
    pub x: f32,
    pub y: f32,
    pub scale: [f32; 2],
    pub rotate: f32,
    pub origin: [f32; 2],
}

/// Всё, что нужно рендеру, уже разобранное и проверенное.
#[derive(Debug, Clone, PartialEq)]
pub struct Visual {
    pub background: Option<Rgba>,
    pub radius: [f32; 4],
    pub border: Option<Border>,
    pub shadows: Vec<Shadow>,
    pub opacity: f32,
    pub clip: bool,
    pub transform: Option<Transform>,
    pub padding: [f32; 4],
}

impl Default for Visual {
    fn default() -> Self {
        Self {
            background: None,
            radius: [0.0; 4],
            border: None,
            shadows: Vec::new(),
            opacity: 1.0,
            clip: false,
            transform: None,
            padding: [0.0; 4],
        }
    }
}

/// Разбирает свойство и превращает ошибку разбора в диагностику.
struct Reader<'a> {
    look: &'a Look,
    path: &'a str,
    src: Option<&'a str>,
    diags: &'a mut Diagnostics,
}

impl Reader<'_> {
    fn read<T>(&mut self, key: &str, parse: impl FnOnce(&Value) -> Result<T, String>) -> Option<T> {
        let resolved = self.look.props.get(key)?;
        match parse(&resolved.value) {
            Ok(v) => Some(v),
            Err(e) => {
                self.diags.push(
                    Level::Error,
                    "look.invalid",
                    self.path,
                    self.src,
                    format!("{key} ({}): {e}", resolved.prov.layer),
                    None,
                );
                None
            }
        }
    }

    fn unsupported(&mut self, key: &str, what: &str) {
        if self.look.props.contains_key(key) {
            self.diags.push(Level::Info, "m0.unsupported", self.path, self.src, format!("{key}: {what}"), None);
        }
    }
}

pub fn visual(look: &Look, path: &str, src: Option<&str>, diags: &mut Diagnostics) -> Visual {
    let mut r = Reader { look, path, src, diags };
    let mut v = Visual::default();

    if let Some((color, gradient)) = r.read("background", parse_paint) {
        if gradient {
            r.diags.push(Level::Info, "m0.unsupported", path, src, "градиент нарисован первым стопом", None);
        }
        v.background = Some(color);
    }
    v.radius = r.read("radius", parse_corners).unwrap_or_default();
    v.padding = r.read("padding", parse_edges).unwrap_or_default();
    v.opacity = r.read("opacity", parse_len).unwrap_or(1.0).clamp(0.0, 1.0);
    v.clip = r.read("clip", |v| v.as_bool().ok_or("ожидается boolean".into())).unwrap_or(false);
    v.border = r.read("border", |b| {
        let width = parse_len(b.get("width").ok_or("border без width")?)?;
        let (color, _) = parse_paint(b.get("paint").ok_or("border без paint")?)?;
        Ok(Border { width, color })
    });
    v.shadows = r
        .read("shadow", |s| {
            let list = match s {
                Value::Array(items) => items.clone(),
                other => vec![other.clone()],
            };
            list.iter()
                .map(|s| {
                    let f = |k: &str| s.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32;
                    Ok(Shadow {
                        x: f("x"),
                        y: f("y"),
                        blur: f("blur"),
                        spread: f("spread"),
                        color: parse_color(s.get("color").ok_or("shadow без color")?)?,
                        inset: s.get("inset").and_then(Value::as_bool).unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    v.transform = r.read("transform", |t| {
        let f = |k: &str| t.get(k).and_then(Value::as_f64).map(|v| v as f32);
        let scale = match t.get("scale") {
            Some(Value::Array(a)) if a.len() == 2 => [parse_len(&a[0])?, parse_len(&a[1])?],
            Some(s) => [parse_len(s)?; 2],
            None => [1.0, 1.0],
        };
        let origin = match t.get("origin").and_then(Value::as_array) {
            Some(o) if o.len() == 2 => [parse_len(&o[0])?, parse_len(&o[1])?],
            _ => [0.5, 0.5],
        };
        Ok(Transform { x: f("x").unwrap_or(0.0), y: f("y").unwrap_or(0.0), scale, rotate: f("rotate").unwrap_or(0.0), origin })
    });
    r.unsupported("blend", "режимы наложения появятся в M1");
    r.unsupported("transition", "переходы исполняются с M2");
    v
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Start,
    Center,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextCase {
    None,
    Upper,
    Lower,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextLook {
    pub family: Option<String>,
    pub size: f32,
    pub weight: f32,
    pub color: Rgba,
    pub line_height: Option<f32>,
    /// Межбуквенный интервал в em.
    pub tracking: f32,
    pub align: TextAlign,
    pub case: TextCase,
}

pub fn text_look(look: &Look, path: &str, src: Option<&str>, diags: &mut Diagnostics) -> TextLook {
    let mut r = Reader { look, path, src, diags };
    TextLook {
        family: r.read("font", |f| {
            f.get("family").and_then(Value::as_str).map(String::from).ok_or_else(|| format!("font без family: {f}"))
        }),
        size: r.read("size", parse_len).unwrap_or(32.0),
        weight: r.read("weight", parse_len).unwrap_or(400.0),
        color: r.read("color", |c| parse_paint(c).map(|(c, _)| c)).unwrap_or(Rgba::WHITE),
        line_height: r.read("lineHeight", parse_len),
        tracking: r.read("tracking", parse_len).unwrap_or(0.0),
        align: r
            .read("align", |a| match a.as_str() {
                Some("start") => Ok(TextAlign::Start),
                Some("center") => Ok(TextAlign::Center),
                Some("end") => Ok(TextAlign::End),
                _ => Err(format!("ожидается start | center | end, получено {a}")),
            })
            .unwrap_or(TextAlign::Start),
        case: r.read("case", parse_case).unwrap_or(TextCase::None),
    }
}

fn parse_case(c: &Value) -> Result<TextCase, String> {
    match c.as_str() {
        Some("none") => Ok(TextCase::None),
        Some("upper") => Ok(TextCase::Upper),
        Some("lower") => Ok(TextCase::Lower),
        _ => Err(format!("ожидается none | upper | lower, получено {c}")),
    }
}

/// Стиль Span: только явно заданные свойства, остальное берётся у Text.
pub fn span_style(look: &Look, path: &str, src: Option<&str>, diags: &mut Diagnostics) -> crate::text::SpanStyle {
    let mut r = Reader { look, path, src, diags };
    crate::text::SpanStyle {
        family: r.read("font", |f| f.get("family").and_then(Value::as_str).map(String::from).ok_or_else(|| format!("font без family: {f}"))),
        size: r.read("size", parse_len),
        weight: r.read("weight", parse_len),
        color: r.read("color", |c| parse_paint(c).map(|(c, _)| c)),
        tracking: r.read("tracking", parse_len),
        case: r.read("case", parse_case),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn edges_and_corners() {
        assert_eq!(parse_edges(&json!(8)).unwrap(), [8.0; 4]);
        assert_eq!(parse_edges(&json!([16, 24])).unwrap(), [16.0, 24.0, 16.0, 24.0]);
        assert!(parse_edges(&json!([1, 2, 3])).is_err());
        assert_eq!(parse_corners(&json!([1, 2, 3, 4])).unwrap(), [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn colors() {
        let c = parse_color(&json!({ "r": 255, "g": 51, "b": 85, "a": 0.5 })).unwrap();
        assert_eq!(c.to_string(), "#ff3355 a0.5");
        // oklch(0.628, 0.2577, 29.23) — это чистый красный sRGB.
        let red = parse_color(&json!({ "oklch": [0.62796, 0.25768, 29.2339] })).unwrap();
        assert_eq!(red.to_u8(), [255, 0, 0, 255]);
    }
}
