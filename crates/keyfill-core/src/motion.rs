//! Движение: кривые и пружины, программы `presence`, переходы `transition` и состояние
//! анимаций сцены.
//!
//! Всё считается аналитически: значение в момент `t` не требует проигрывать кадры до него,
//! нужно только обработать события до `t` (команды сценария, окончания фаз). Пружины решаются
//! в замкнутой форме (затухающий осциллятор), поэтому кадр детерминирован и не зависит от того,
//! какие кадры рендерились перед ним.
//!
//! Два слоя значений:
//! - **presence** (`in`, `loop`, `out`) работает поверх `look`: `x`, `y`, `rotate` — сдвиг,
//!   `scale`, `opacity` — множитель, `reveal` — доля видимых символов; остальные каналы
//!   перекрывают значение `look`. Этот слой не участвует в `must`: проверяется состояние
//!   графики в покое, а не промежуточный кадр появления.
//! - **transition** анимирует само значение `look`, когда данные изменились.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::diag::{Diagnostics, Level};
use crate::dynamic::Dyn;
use crate::look::{self, Rgba, num};
use crate::scene::{Kind, MotionAction, Scene};
use crate::timeline::Run;

// ─── Каналы и значения ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Channel {
    X,
    Y,
    Scale,
    Rotate,
    Opacity,
    Width,
    Height,
    Radius,
    Background,
    Color,
    Reveal,
}

impl Channel {
    pub fn parse(key: &str) -> Option<Channel> {
        Some(match key {
            "x" => Channel::X,
            "y" => Channel::Y,
            "scale" => Channel::Scale,
            "rotate" => Channel::Rotate,
            "opacity" => Channel::Opacity,
            "width" => Channel::Width,
            "height" => Channel::Height,
            "radius" => Channel::Radius,
            "background" => Channel::Background,
            "color" => Channel::Color,
            "reveal" => Channel::Reveal,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Channel::X => "x",
            Channel::Y => "y",
            Channel::Scale => "scale",
            Channel::Rotate => "rotate",
            Channel::Opacity => "opacity",
            Channel::Width => "width",
            Channel::Height => "height",
            Channel::Radius => "radius",
            Channel::Background => "background",
            Channel::Color => "color",
            Channel::Reveal => "reveal",
        }
    }

    /// Нейтральное значение канала в слое presence: сдвиг 0 или множитель 1.
    /// `None` — канал перекрывает значение `look`.
    pub fn identity(self) -> Option<f64> {
        match self {
            Channel::X | Channel::Y | Channel::Rotate => Some(0.0),
            Channel::Scale | Channel::Opacity | Channel::Reveal => Some(1.0),
            _ => None,
        }
    }

    fn is_color(self) -> bool {
        matches!(self, Channel::Background | Channel::Color)
    }
}

/// Значение канала: число или цвет (sRGB, каналы 0..1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum V {
    N(f64),
    C([f64; 4]),
}

impl V {
    pub fn n(self) -> f64 {
        match self {
            V::N(v) => v,
            V::C(_) => 0.0,
        }
    }

    pub fn rgba(self) -> Rgba {
        match self {
            V::C([r, g, b, a]) => Rgba { r: r as f32, g: g as f32, b: b as f32, a: a as f32 },
            V::N(_) => Rgba::default(),
        }
    }

    fn from_rgba(c: Rgba) -> V {
        V::C([c.r as f64, c.g as f64, c.b as f64, c.a as f64])
    }

    fn lerp(a: V, b: V, p: f64) -> V {
        match (a, b) {
            (V::N(x), V::N(y)) => V::N(x + (y - x) * p),
            (V::C(x), V::C(y)) => V::C(std::array::from_fn(|i| x[i] + (y[i] - x[i]) * p)),
            (_, b) => b,
        }
    }

    pub fn show(self) -> String {
        match self {
            V::N(v) => num(v as f32),
            V::C(_) => self.rgba().to_string(),
        }
    }
}

// ─── Изинги ─────────────────────────────────────────────────────────────────

const LINEAR: [f64; 4] = [0.0, 0.0, 1.0, 1.0];
const EASE_IN: [f64; 4] = [0.42, 0.0, 1.0, 1.0];
const EASE_OUT: [f64; 4] = [0.0, 0.0, 0.58, 1.0];
pub(crate) const EASE_IN_OUT: [f64; 4] = [0.42, 0.0, 0.58, 1.0];

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Easing {
    /// Кривая Безье `[x1, y1, x2, y2]` за фиксированное время.
    Tween { duration: f64, curve: [f64; 4] },
    Spring { stiffness: f64, damping: f64, mass: f64 },
}

impl Easing {
    pub fn parse(v: &Value) -> Result<Easing, String> {
        let f = |k: &str| v.get(k).and_then(Value::as_f64);
        match v.get("kind").and_then(Value::as_str) {
            Some("tween") => {
                let duration = f("duration").filter(|d| *d >= 0.0).ok_or("tween: ожидается duration ≥ 0")?;
                let curve = match v.get("ease") {
                    None => EASE_IN_OUT,
                    Some(Value::String(s)) => match s.as_str() {
                        "linear" => LINEAR,
                        "in" => EASE_IN,
                        "out" => EASE_OUT,
                        "inOut" => EASE_IN_OUT,
                        other => return Err(format!("tween: неизвестная кривая {other:?}")),
                    },
                    Some(Value::Array(c)) if c.len() == 4 => {
                        let c: Vec<f64> = c.iter().map(|x| x.as_f64().ok_or("tween: кривая из чисел")).collect::<Result<_, _>>()?;
                        [c[0], c[1], c[2], c[3]]
                    }
                    Some(other) => return Err(format!("tween: кривая {other} не разобрана")),
                };
                Ok(Easing::Tween { duration, curve })
            }
            Some("spring") => {
                let stiffness = f("stiffness").filter(|v| *v > 0.0).ok_or("spring: ожидается stiffness > 0")?;
                let damping = f("damping").filter(|v| *v >= 0.0).ok_or("spring: ожидается damping ≥ 0")?;
                let mass = f("mass").unwrap_or(1.0);
                if mass <= 0.0 {
                    return Err("spring: ожидается mass > 0".into());
                }
                if damping == 0.0 {
                    return Err("spring без затухания никогда не остановится: задай damping > 0".into());
                }
                Ok(Easing::Spring { stiffness, damping, mass })
            }
            _ => Err(format!("изинг: ожидается tween(...) или spring(...), получено {v}")),
        }
    }

    /// Прогресс 0..1 (пружина может перелетать) через `t` секунд от старта. `v0` — начальная
    /// скорость прогресса в долях в секунду; её передаёт прерванная пружина.
    pub fn progress(&self, t: f64, v0: f64) -> f64 {
        match *self {
            Easing::Tween { duration, curve } => {
                if duration <= 0.0 {
                    return 1.0;
                }
                bezier(curve, (t / duration).clamp(0.0, 1.0))
            }
            Easing::Spring { stiffness, damping, mass } => spring_state(stiffness, damping, mass, v0, t).0,
        }
    }

    /// Длительность: у tween — заданная, у пружины — время, когда она успокоилась.
    pub fn duration(&self, v0: f64) -> f64 {
        match *self {
            Easing::Tween { duration, .. } => duration,
            Easing::Spring { stiffness, damping, mass } => spring_settle(stiffness, damping, mass, v0),
        }
    }

    pub fn label(&self) -> String {
        match *self {
            Easing::Tween { duration, curve } => {
                let name = match curve {
                    LINEAR => "linear".to_string(),
                    EASE_IN => "in".to_string(),
                    EASE_OUT => "out".to_string(),
                    EASE_IN_OUT => "inOut".to_string(),
                    c => format!("[{}]", c.map(|v| num(v as f32)).join(",")),
                };
                format!("tween {}s {name}", num(duration as f32))
            }
            Easing::Spring { stiffness, damping, mass } => {
                let preset = match (stiffness, damping, mass) {
                    (400.0, 30.0, 1.0) => " (snappy)",
                    (120.0, 20.0, 1.0) => " (gentle)",
                    (300.0, 12.0, 1.0) => " (bouncy)",
                    _ => "",
                };
                format!("spring {}/{}/{}{preset}", num(stiffness as f32), num(damping as f32), num(mass as f32))
            }
        }
    }
}

/// cubic-bezier(x1, y1, x2, y2) как в CSS: по доле времени `x` находит долю значения.
fn bezier(c: [f64; 4], x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let [x1, y1, x2, y2] = c;
    if x1 == y1 && x2 == y2 {
        return x;
    }
    let curve = |a: f64, b: f64, t: f64| 3.0 * a * t * (1.0 - t).powi(2) + 3.0 * b * t * t * (1.0 - t) + t.powi(3);
    let slope = |t: f64| 3.0 * x1 * (1.0 - t).powi(2) + 6.0 * (x2 - x1) * t * (1.0 - t) + 3.0 * (1.0 - x2) * t * t;
    let mut t = x;
    for _ in 0..8 {
        let d = curve(x1, x2, t) - x;
        let s = slope(t);
        if d.abs() < 1e-10 || s.abs() < 1e-9 {
            break;
        }
        t -= d / s;
    }
    if !(0.0..=1.0).contains(&t) || (curve(x1, x2, t) - x).abs() > 1e-7 {
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..60 {
            t = (lo + hi) / 2.0;
            if curve(x1, x2, t) < x {
                lo = t;
            } else {
                hi = t;
            }
        }
    }
    curve(y1, y2, t)
}

/// Прогресс и его скорость у пружины, которая идёт от 0 к 1 с начальной скоростью `v0`.
/// Замкнутая форма затухающего осциллятора для отклонения `x = 1 - p`.
fn spring_state(k: f64, c: f64, m: f64, v0: f64, t: f64) -> (f64, f64) {
    let w0 = (k / m).sqrt();
    let z = c / (2.0 * (k * m).sqrt());
    let (x0, xv0) = (1.0, -v0);
    let (x, v) = if (z - 1.0).abs() < 1e-6 {
        let b = xv0 + w0 * x0;
        let e = (-w0 * t).exp();
        (e * (x0 + b * t), e * (b - w0 * (x0 + b * t)))
    } else if z < 1.0 {
        let wd = w0 * (1.0 - z * z).sqrt();
        let e = (-z * w0 * t).exp();
        let (s, co) = (wd * t).sin_cos();
        let b = (xv0 + z * w0 * x0) / wd;
        let x = e * (x0 * co + b * s);
        (x, -z * w0 * x + e * (-x0 * wd * s + b * wd * co))
    } else {
        let s = (z * z - 1.0).sqrt();
        let (r1, r2) = (-w0 * (z - s), -w0 * (z + s));
        let c1 = (xv0 - r2 * x0) / (r1 - r2);
        let c2 = x0 - c1;
        let (e1, e2) = ((r1 * t).exp(), (r2 * t).exp());
        (c1 * e1 + c2 * e2, c1 * r1 * e1 + c2 * r2 * e2)
    };
    (1.0 - x, -v)
}

/// Время, после которого пружина в пределах 0.1% от цели и почти неподвижна. Первые 10 с
/// проверяются с шагом 1 мс; медленная пружина ищется дальше, до горизонта по скорости
/// затухания огибающей, — без этого она обрывалась скачком на 10 с.
fn spring_settle(k: f64, c: f64, m: f64, v0: f64) -> f64 {
    let moving = |t: f64| {
        let (p, dp) = spring_state(k, c, m, v0, t);
        (1.0 - p).abs() > 1e-3 || dp.abs() > 1e-2
    };
    const FINE: u32 = 10_000;
    let mut last = 0;
    for i in 0..=FINE {
        if moving(f64::from(i) / 1000.0) {
            last = i;
        }
    }
    if last < FINE {
        return f64::from(last + 1) / 1000.0;
    }
    let w0 = (k / m).sqrt();
    let z = c / (2.0 * (k * m).sqrt());
    let rate = if z < 1.0 { z * w0 } else { w0 * (z - (z * z - 1.0).max(0.0).sqrt()) };
    // ln(1000) огибающей с запасом на начальную скорость и множитель t критического режима.
    let horizon = (((1000.0 * (1.0 + v0.abs() / w0)).ln() + 8.0) / rate.max(1e-6)).clamp(10.0, SPRING_LIMIT);
    const COARSE: u32 = 20_000;
    let step = (horizon - 10.0) / f64::from(COARSE);
    let mut end = 10.0;
    for i in 1..=COARSE {
        let t = 10.0 + step * f64::from(i);
        if moving(t) {
            end = t + step;
        }
    }
    end.min(SPRING_LIMIT)
}

/// Дольше пружина не считается: её движение обрывается на этом моменте.
const SPRING_LIMIT: f64 = 3600.0;

// ─── Сегменты и дорожки ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Seg {
    pub start: f64,
    pub from: V,
    pub to: V,
    pub easing: Easing,
    /// Начальная скорость прогресса (только пружины, при прерывании).
    pub v0: f64,
    pub dur: f64,
    /// Бесконечное качание from → to → from (pulse).
    pub pingpong: bool,
}

impl Seg {
    pub(crate) fn new(start: f64, from: V, to: V, easing: Easing, v0: f64, pingpong: bool) -> Seg {
        Seg { start, from, to, easing, v0, dur: easing.duration(v0), pingpong }
    }

    fn progress(&self, t: f64) -> f64 {
        let local = (t - self.start).max(0.0);
        if self.pingpong {
            if self.dur <= 0.0 {
                return 1.0;
            }
            let cycle = local % (2.0 * self.dur);
            return if cycle < self.dur {
                self.easing.progress(cycle, 0.0)
            } else {
                1.0 - self.easing.progress(cycle - self.dur, 0.0)
            };
        }
        if local >= self.dur { 1.0 } else { self.easing.progress(local, self.v0) }
    }

    fn value(&self, t: f64) -> V {
        V::lerp(self.from, self.to, self.progress(t))
    }
}

/// Значения одного канала во времени: сегменты по возрастанию старта.
#[derive(Debug, Clone)]
pub struct Track {
    pub segs: Vec<Seg>,
    /// Что запустило дорожку: `in`, `loop`, `out`, `transition`.
    pub source: &'static str,
}

impl Track {
    pub(crate) fn active(&self, t: f64) -> Option<&Seg> {
        self.segs.iter().rev().find(|s| s.start <= t)
    }

    pub fn value(&self, t: f64) -> V {
        match self.active(t) {
            Some(s) => s.value(t),
            None => self.segs[0].from,
        }
    }

    /// Скорость значения в единицах канала в секунду (для передачи импульса пружине).
    fn velocity(&self, t: f64) -> f64 {
        let Some(s) = self.active(t) else { return 0.0 };
        let h = 1e-4;
        match (s.value(t + h), s.value(t)) {
            (V::N(a), V::N(b)) => (a - b) / h,
            _ => 0.0,
        }
    }

    pub fn end(&self) -> Option<f64> {
        if self.segs.iter().any(|s| s.pingpong) {
            return None;
        }
        self.segs.iter().map(|s| s.start + s.dur).reduce(f64::max)
    }

    pub(crate) fn running(&self, t: f64) -> bool {
        self.end().is_none_or(|e| t < e) && self.segs.first().is_some_and(|s| s.start <= t)
    }

    pub(crate) fn easing_label(&self, t: f64) -> String {
        self.active(t).or(self.segs.first()).map(|s| s.easing.label()).unwrap_or_default()
    }
}

// ─── Программы presence ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ProgSeg {
    pub channel: Channel,
    pub offset: f64,
    /// Явное начальное значение; без него движение идёт от текущего.
    pub from: Option<V>,
    pub to: V,
    pub easing: Easing,
    pub pingpong: bool,
}

/// Скомпилированное движение (`animate`, `seq`, `par`, `delay`, `pulse`, `stagger`): сегменты по каналам.
#[derive(Debug, Clone, Default)]
pub struct Program {
    pub segs: Vec<ProgSeg>,
    /// Длительность без каскада: сколько детей, станет известно только при запуске.
    pub duration: f64,
    pub infinite: bool,
    /// `stagger`: движение для каждого ребёнка узла со сдвигом `i·step`.
    pub per_child: Vec<StaggerPart>,
}

#[derive(Debug, Clone)]
pub struct StaggerPart {
    pub offset: f64,
    pub step: f64,
    pub reverse: bool,
    pub program: Program,
}

impl Program {
    fn shifted(mut self, by: f64) -> Program {
        for s in &mut self.segs {
            s.offset += by;
        }
        for p in &mut self.per_child {
            p.offset += by;
        }
        self.duration += by;
        self
    }

    fn merge(&mut self, other: Program) {
        self.segs.extend(other.segs);
        self.per_child.extend(other.per_child);
        self.duration = self.duration.max(other.duration);
        self.infinite |= other.infinite;
    }
}

pub(crate) struct Compiler<'a> {
    pub(crate) theme: &'a BTreeMap<String, Value>,
    pub(crate) notes: Vec<(Level, &'static str, String)>,
}

impl Compiler<'_> {
    pub(crate) fn value(&mut self, ch: Channel, raw: &Value) -> Result<V, String> {
        if ch.is_color() {
            let (mut tokens, mut missing) = (Vec::new(), Vec::new());
            let v = look::substitute(raw, self.theme, &mut tokens, &mut missing);
            if let Some(m) = missing.first() {
                return Err(format!("токен {m} не найден в теме"));
            }
            return look::parse_color(&v).map(V::from_rgba);
        }
        raw.as_f64().map(V::N).ok_or_else(|| format!("{}: ожидается число, получено {raw}", ch.name()))
    }

    fn compile(&mut self, v: &Value) -> Result<Program, String> {
        if let Some(items) = v.as_array() {
            return self.par(items);
        }
        match v.get("motion").and_then(Value::as_str) {
            Some("animate") => {
                let easing = Easing::parse(v.get("easing").ok_or("animate без easing")?)?;
                let settle = easing.duration(0.0);
                if matches!(easing, Easing::Spring { .. }) && settle > 10.0 {
                    let msg = format!("пружина успокаивается {} с: поставь stiffness выше или damping ближе к критическому", num(settle as f32));
                    self.notes.push((Level::Warn, "motion.spring-slow", msg));
                }
                let targets = v.get("targets").and_then(Value::as_object).ok_or("animate без targets")?;
                let mut segs = Vec::new();
                for (key, raw) in targets {
                    let Some(channel) = Channel::parse(key) else {
                        let what = if key == "blur" { "blur пока не анимируется" } else { "неизвестный канал" };
                        self.notes.push((Level::Warn, "motion.unsupported", format!("{key}: {what}, пропущен")));
                        continue;
                    };
                    let (from, to) = match raw.as_array() {
                        Some(pair) if pair.len() == 2 => (Some(self.value(channel, &pair[0])?), self.value(channel, &pair[1])?),
                        _ => (None, self.value(channel, raw)?),
                    };
                    segs.push(ProgSeg { channel, offset: 0.0, from, to, easing, pingpong: false });
                }
                Ok(Program { segs, duration: easing.duration(0.0), ..Program::default() })
            }
            Some("seq") => {
                let items = v.get("items").and_then(Value::as_array).ok_or("seq без items")?;
                let mut out = Program::default();
                for (i, item) in items.iter().enumerate() {
                    if out.infinite {
                        self.notes.push((Level::Warn, "motion.unreachable", format!("seq: шаг {i} идёт после бесконечного и не начнётся")));
                        break;
                    }
                    let p = self.compile(item)?.shifted(out.duration);
                    out.segs.extend(p.segs);
                    out.per_child.extend(p.per_child);
                    out.duration = p.duration;
                    out.infinite = p.infinite;
                }
                Ok(out)
            }
            Some("par") => self.par(v.get("items").and_then(Value::as_array).ok_or("par без items")?),
            Some("delay") => {
                let s = v.get("seconds").and_then(Value::as_f64).filter(|s| *s >= 0.0).ok_or("delay: ожидается seconds ≥ 0")?;
                Ok(self.compile(v.get("item").ok_or("delay без движения")?)?.shifted(s))
            }
            Some("stagger") => {
                let step = v.get("step").and_then(Value::as_f64).filter(|s| *s >= 0.0).ok_or("stagger: ожидается step ≥ 0")?;
                let program = self.compile(v.get("item").ok_or("stagger без движения")?)?;
                if !program.per_child.is_empty() {
                    return Err("stagger внутри stagger не поддерживается".into());
                }
                let reverse = v.get("reverse").and_then(Value::as_bool).unwrap_or(false);
                Ok(Program { per_child: vec![StaggerPart { offset: 0.0, step, reverse, program }], ..Program::default() })
            }
            Some("pulse") => {
                let period = v.get("period").and_then(Value::as_f64).filter(|p| *p > 0.0).ok_or("pulse: ожидается period > 0")?;
                let easing = Easing::Tween { duration: period / 2.0, curve: EASE_IN_OUT };
                let mut segs = Vec::new();
                for (key, channel) in [("opacity", Channel::Opacity), ("scale", Channel::Scale)] {
                    if let Some(pair) = v.get(key).and_then(Value::as_array).filter(|p| p.len() == 2) {
                        let (from, to) = (self.value(channel, &pair[0])?, self.value(channel, &pair[1])?);
                        segs.push(ProgSeg { channel, offset: 0.0, from: Some(from), to, easing, pingpong: true });
                    }
                }
                Ok(Program { segs, duration: period, infinite: true, ..Program::default() })
            }
            _ => Err(format!("не движение: {v}")),
        }
    }

    fn par(&mut self, items: &[Value]) -> Result<Program, String> {
        let mut out = Program::default();
        for item in items {
            out.merge(self.compile(item)?);
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Presence {
    pub enter: Option<Program>,
    pub repeat: Option<Program>,
    pub exit: Option<Program>,
}

// ─── Состояние анимаций сцены ───────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Default)]
enum Phase {
    /// Сцена загружена, play ещё не было: узел стоит в начальном кадре `in`.
    Waiting,
    In { start: f64, end: f64 },
    Loop,
    /// На экране, движения presence нет.
    #[default]
    Shown,
    Out { start: f64, end: f64 },
    /// Ушёл после `out`.
    Gone,
}

#[derive(Debug, Clone, Default)]
struct NodeState {
    presence: Option<Presence>,
    phase: Phase,
    rel: BTreeMap<Channel, Track>,
    abs: BTreeMap<Channel, Track>,
    /// Значения из `look` и размеров: к ним ведут переходы.
    targets: BTreeMap<Channel, V>,
    visible: bool,
    own_hidden: bool,
    /// Дети в порядке сцены: для каскада `stagger`.
    children: Vec<u32>,
    /// Экземпляр снят адресным stop: с этого момента не рисуется.
    hide_at: Option<f64>,
    /// Событие снятия в `hide_at` обработано: узла больше нет в кадре.
    stopped: bool,
    /// `transition.layout`: как узел переезжает, когда перестроилась раскладка.
    layout: Option<Easing>,
    /// Смещение от новой раскладки к месту, где узел был виден, затухает до нуля: x, y, w, h.
    geo: [Option<Track>; 4],
}

impl NodeState {
    fn abs_value(&self, ch: Channel, t: f64) -> V {
        if let Some(tr) = self.abs.get(&ch) {
            return tr.value(t);
        }
        self.targets.get(&ch).copied().unwrap_or(if ch.is_color() { V::C([0.0; 4]) } else { V::N(0.0) })
    }

    fn current(&self, ch: Channel, t: f64) -> V {
        if let Some(tr) = self.rel.get(&ch) {
            return tr.value(t);
        }
        match ch.identity() {
            Some(v) => V::N(v),
            None => self.abs_value(ch, t),
        }
    }

    /// Запускает программу на слое presence. `continuing` — узел на экране: первый сегмент
    /// каждого канала идёт от текущего значения, пружина сохраняет скорость.
    fn start(&mut self, prog: &Program, t: f64, source: &'static str, continuing: bool) {
        // Узел, которого не было на экране, начинает с объявленных значений: дорожки прошлого
        // `out` (например, сдвиг) не должны пережить повторное появление.
        if !continuing {
            self.rel.clear();
        }
        let mut by_channel: BTreeMap<Channel, Vec<&ProgSeg>> = BTreeMap::new();
        for s in &prog.segs {
            by_channel.entry(s.channel).or_default().push(s);
        }
        for (ch, mut segs) in by_channel {
            segs.sort_by(|a, b| a.offset.total_cmp(&b.offset));
            let current = self.current(ch, t);
            let existing = self.rel.get(&ch).filter(|_| continuing);
            let mut out = Vec::with_capacity(segs.len());
            let mut prev_to = None;
            for (i, ps) in segs.iter().enumerate() {
                let from = if ps.pingpong {
                    ps.from.unwrap_or(current)
                } else if i == 0 && existing.is_some() {
                    current
                } else {
                    ps.from.or(prev_to).unwrap_or(current)
                };
                let v0 = match (existing, ps.easing, from, ps.to) {
                    (Some(tr), Easing::Spring { .. }, V::N(a), V::N(b)) if i == 0 && (b - a).abs() > 1e-9 => tr.velocity(t) / (b - a),
                    _ => 0.0,
                };
                out.push(Seg::new(t + ps.offset, from, ps.to, ps.easing, v0, ps.pingpong));
                prev_to = Some(ps.to);
            }
            self.rel.insert(ch, Track { segs: out, source });
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize)]
#[serde(into = "&'static str")]
pub enum ScenePhase {
    /// Загружена, ждёт play.
    #[default]
    Loaded,
    Playing,
    Stopping { until: f64 },
    /// Снята: все `out` закончились.
    Removed,
}

impl From<ScenePhase> for &'static str {
    fn from(p: ScenePhase) -> &'static str {
        p.label()
    }
}

impl std::fmt::Display for ScenePhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

impl ScenePhase {
    pub fn label(self) -> &'static str {
        match self {
            ScenePhase::Loaded => "loaded",
            ScenePhase::Playing => "playing",
            ScenePhase::Stopping { .. } => "stopping",
            ScenePhase::Removed => "removed",
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Event {
    InEnd(u32),
    OutEnd(u32),
    /// Экземпляр, снятый адресным stop, уходит из кадра.
    Hide(u32),
    Removed,
    /// Остановка таймлайна `runs[i]` с поколением расписания.
    Timeline(usize, u64),
}

/// Событие для пульта и журнала: смена состояния, маркер, своё событие, конец.
#[derive(Debug, Clone, PartialEq)]
pub struct MotionEvent {
    pub t: f64,
    /// `scene` или экземпляр (`Scoreboard#1`).
    pub source: String,
    pub kind: &'static str,
    pub name: String,
}

impl MotionEvent {
    pub fn label(&self) -> String {
        format!("{}s {} {}:{}", num(self.t as f32), self.source, self.kind, self.name)
    }
}

/// Анимационное состояние сцены. События обрабатываются строго по времени, поэтому
/// кадр в момент `t` один и тот же, как бы мы к нему ни пришли.
#[derive(Debug, Clone, Default)]
pub struct MotionState {
    nodes: BTreeMap<u32, NodeState>,
    /// (время, порядковый номер, событие), по возрастанию.
    events: Vec<(f64, u64, Event)>,
    seq: u64,
    scene: ScenePhase,
    initialized: bool,
    /// Канал: экземпляры появляются и уходят командами, сцена играет с момента 0.
    channel: bool,
    /// Корни экземпляров, загруженных на слой и ещё не показанных.
    pending: BTreeSet<u32>,
    /// Корни экземпляров, для которых уже созданы воспроизведения таймлайнов.
    with_runs: BTreeSet<u32>,
    /// Воспроизведения таймлайнов, по одному на экземпляр компонента.
    runs: Vec<Run>,
    /// `count()` и `clock()`: (узел Text, номер куска) → состояние.
    dyns: BTreeMap<(u32, usize), Dyn>,
    /// Журнал команд для дампа: `play@0`, `stop@2.5`.
    pub log: Vec<String>,
    /// События по времени: состояния, маркеры, emit.
    pub events_log: Vec<MotionEvent>,
    pub diagnostics: Diagnostics,
    /// Когда закончатся все `in`, начатые последним play: кадр «в эфире» по умолчанию.
    pub settled_at: Option<f64>,
    /// Последний момент, до которого что-то запланировано (для раскадровки по умолчанию).
    pub horizon: f64,
}

/// Значения анимаций узла в момент `t`.
#[derive(Debug, Clone, Default)]
pub struct NodeValues {
    /// Каналы, перекрывающие значение `look` (идущий переход или presence по такому каналу).
    pub abs: BTreeMap<Channel, V>,
    pub motion: RelMotion,
    /// `Some(true)` — рисовать, хотя узел скрыт (идёт `out`); `Some(false)` — узел ушёл.
    pub show: Option<bool>,
    /// Почему узел ушёл, если `show == Some(false)`.
    pub gone: Option<Gone>,
    /// Каналы `abs`, которые сейчас ведёт presence или таймлайн (идёт их движение): кадр
    /// рисуется с ними, а `must` проверяет графику без них — это промежуточное состояние.
    pub staged: Vec<Channel>,
    /// Что с узлом происходит: `in 78%`, `loop`, `transition opacity 40%`.
    pub notes: Vec<Note>,
    /// Подробности по каналам для inspect.
    pub channels: Vec<ChannelInfo>,
    /// Текст кусков `count()` и `clock()` по номеру куска.
    pub texts: BTreeMap<usize, String>,
    /// Анимация геометрии: смещение места и размера от раскладки (x, y, w, h).
    pub geo: Option<[f32; 4]>,
}

/// Геометрия узла в раскладке: место относительно родителя и размер (x, y, w, h).
/// `sized` — размер анимируется; у текста только место: он не переразбивается на кадрах.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geo {
    pub rel: [f32; 4],
    pub sized: bool,
}

pub type GeoMap = BTreeMap<u32, Geo>;

/// Канал узла для inspect: значение, слой (`presence`, `transition`, `timeline`), изинг, прогресс.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ChannelInfo {
    pub channel: &'static str,
    pub value: String,
    pub layer: &'static str,
    pub source: &'static str,
    pub easing: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
}

/// Фаза узла в момент кадра. Текст — пометка дампа, JSON — структура с прогрессом.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Note {
    /// Ждёт play сцены.
    Waiting,
    /// `progress` — доля от 0 до 1; `None` у бесконечного движения.
    In { progress: Option<f64> },
    Loop,
    Out { progress: Option<f64> },
    Gone,
    Transition { channel: &'static str, progress: Option<f64> },
    Timeline { state: crate::timeline::RunState, tau: f64, marker: Option<String> },
    /// Идёт анимация `count()`.
    Count,
    /// Узел переезжает к новой раскладке (`transition.layout`).
    Layout { progress: Option<f64> },
}

impl std::fmt::Display for Note {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Note::Waiting => f.write_str("ждёт play"),
            Note::In { progress } => write!(f, "in {}%", pct(*progress)),
            Note::Loop => f.write_str("loop"),
            Note::Out { progress } => write!(f, "out {}%", pct(*progress)),
            Note::Gone => f.write_str("ушёл"),
            Note::Transition { channel, progress } => write!(f, "transition {channel} {}%", pct(*progress)),
            Note::Timeline { state, tau, marker } => {
                write!(f, "timeline {} τ={}", state.label(), num(*tau as f32))?;
                if let Some(m) = marker {
                    write!(f, " на {m}")?;
                }
                Ok(())
            }
            Note::Count => f.write_str("count"),
            Note::Layout { progress } => write!(f, "layout {}%", pct(*progress)),
        }
    }
}

/// Почему узла больше нет в кадре.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Gone {
    /// Закончил свой `out`.
    OutEnded,
    /// Экземпляр снят адресным `stop`.
    InstanceStopped,
    /// Таймлайн экземпляра доиграл `out` и снял его.
    TimelineRemoved,
    /// Экземпляр загружен на слой канала (`load`) и ждёт play.
    Loaded,
}

/// Слой presence: сдвиги и множители поверх `look`, не участвуют в `must`.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct RelMotion {
    pub dx: f32,
    pub dy: f32,
    pub scale: f32,
    pub rotate: f32,
    pub opacity: f32,
    /// Доля видимых символов текста.
    pub reveal: f32,
}

impl Default for RelMotion {
    fn default() -> Self {
        Self { dx: 0.0, dy: 0.0, scale: 1.0, rotate: 0.0, opacity: 1.0, reveal: 1.0 }
    }
}

impl RelMotion {
    pub fn is_identity(&self) -> bool {
        *self == RelMotion::default()
    }

    pub fn describe(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.opacity != 1.0 {
            out.push(format!("opacity×{}", num(self.opacity)));
        }
        if self.dx != 0.0 || self.dy != 0.0 {
            out.push(format!("сдвиг {},{}", num(self.dx), num(self.dy)));
        }
        if self.scale != 1.0 {
            out.push(format!("scale×{}", num(self.scale)));
        }
        if self.rotate != 0.0 {
            out.push(format!("rotate {}°", num(self.rotate)));
        }
        if self.reveal != 1.0 {
            out.push(format!("reveal {}", num(self.reveal)));
        }
        out
    }
}

/// Время кадра для дампа.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Clock {
    pub t: f64,
    pub frame: u64,
    pub scene: ScenePhase,
    /// Обработанные команды сценария.
    pub log: Vec<String>,
    /// События до момента кадра включительно.
    pub events: Vec<String>,
    /// Расписание перехода канала, пока он идёт.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition: Option<crate::transition::TransitionPlan>,
}

/// Всё, что движение добавляет к кадру.
#[derive(Debug, Clone, Default)]
pub struct Animated {
    pub nodes: BTreeMap<u32, NodeValues>,
    /// Сцена снята: в кадре ничего нет.
    pub removed: bool,
    pub clock: Option<Clock>,
    pub diagnostics: Diagnostics,
}

impl Animated {
    /// Есть ли каналы в движении presence или таймлайна.
    pub fn has_staged(&self) -> bool {
        self.nodes.values().any(|n| !n.staged.is_empty())
    }

    /// Убирает анимацию геометрии: по таким значениям снимается раскладка до и после перестройки.
    pub fn clear_geo(&mut self) {
        for nv in self.nodes.values_mut() {
            nv.geo = None;
        }
    }

    /// Те же значения без каналов в движении presence и таймлайна: по ним проверяется `must`.
    pub fn without_staged(&self) -> Animated {
        let mut out = self.clone();
        for nv in out.nodes.values_mut() {
            for ch in std::mem::take(&mut nv.staged) {
                nv.abs.remove(&ch);
            }
        }
        out
    }
}

impl MotionState {
    pub fn scene_phase(&self) -> ScenePhase {
        self.scene
    }

    fn schedule(&mut self, t: f64, ev: Event) {
        self.seq += 1;
        let key = (t, self.seq);
        let pos = self.events.partition_point(|(et, s, _)| (*et, *s) < key);
        self.events.insert(pos, (t, self.seq, ev));
        self.horizon = self.horizon.max(t);
    }

    fn note(&mut self, level: Level, rule: &str, scene: &Scene, id: u32, message: String) {
        let path = scene.path(id);
        if !self.diagnostics.0.iter().any(|d| d.rule == rule && d.node == path && d.message == message) {
            self.diagnostics.push(level, rule, &path, scene.node(id).src.as_deref(), message, None);
        }
    }

    /// Сверяет состояние с деревом сцены после команд JS в момент `t`: запускает переходы
    /// изменившихся значений и `in`/`out` узлов, которые появились или скрылись.
    pub fn observe(&mut self, scene: &Scene, t: f64) {
        let visible = effective_visibility(scene);
        let mut enter = Vec::new();
        let mut leave = Vec::new();
        // Что было до этих команд: новые узлы под ними — элементы, добавленные данными (For).
        let known: BTreeSet<u32> = self.nodes.keys().copied().collect();
        let mut fresh = Vec::new();
        let with_timeline: Vec<String> = scene.timelines.iter().map(|d| d.component.clone()).collect();
        let ids: Vec<u32> = scene.nodes.keys().copied().collect();
        for id in ids {
            let node = scene.node(id);
            if node.kind == Kind::Stage {
                continue;
            }
            let is_new = !self.nodes.contains_key(&id);
            if is_new {
                fresh.push(id);
                let mut presence_raw = node.props.get("presence");
                // Корнем компонента с таймлайном управляет таймлайн, а не presence.
                if presence_raw.is_some()
                    && let Some(inst) = node.instance()
                    && with_timeline.iter().any(|c| c == component_of(inst))
                {
                    presence_raw = None;
                    self.note(Level::Info, "motion.timeline", scene, id, "presence корня не исполняется: появлением и уходом управляет таймлайн".into());
                }
                let presence = presence_raw.map(|p| self.compile_presence(scene, id, p));
                let mut st = NodeState { presence, children: node.children.clone(), ..NodeState::default() };
                let enter = st.presence.as_ref().map(|p| p.enter.clone());
                if enter.is_some() {
                    st.phase = Phase::Waiting;
                }
                st.visible = visible.get(&id).copied().unwrap_or(false);
                self.nodes.insert(id, st);
                // Пока не было play, узел (и каскад его детей) стоит в первом кадре своего in.
                if let Some(Some(enter)) = enter {
                    self.run_program(id, &enter, f64::INFINITY, "in", false);
                }
            }

            let (targets, transitions) = self.targets_of(scene, id);
            let st = self.nodes.get_mut(&id).expect("node state");
            // Дети меняются: элементы списка добавляются, уходят и переставляются.
            st.children.clone_from(&node.children);
            for (&ch, &new) in &targets {
                let Some(&old) = st.targets.get(&ch) else { continue };
                if old == new || is_new {
                    continue;
                }
                let easing = transitions.get(ch.name()).or_else(|| transitions.get("all")).copied();
                match easing {
                    Some(easing) => {
                        let from = st.abs_value(ch, t);
                        let (from, to) = fade_colors(from, new);
                        let v0 = match (st.abs.get(&ch), easing, from, to) {
                            (Some(tr), Easing::Spring { .. }, V::N(a), V::N(b)) if (b - a).abs() > 1e-9 => tr.velocity(t) / (b - a),
                            _ => 0.0,
                        };
                        st.abs.insert(ch, Track { segs: vec![Seg::new(t, from, to, easing, v0, false)], source: "transition" });
                    }
                    None => {
                        st.abs.remove(&ch);
                    }
                }
            }
            st.targets = targets;
            st.layout = transitions.get("layout").copied();

            let vis = visible.get(&id).copied().unwrap_or(false);
            let hidden = node.hidden();
            if !is_new && self.scene == ScenePhase::Playing {
                if vis && !st.visible {
                    enter.push(id);
                } else if hidden && !st.own_hidden {
                    leave.push(id);
                }
            }
            st.visible = vis;
            st.own_hidden = hidden;

            for (i, run) in node.runs.iter().enumerate() {
                let Some(desc) = &run.dynamic else { continue };
                match Dyn::update(self.dyns.get(&(id, i)), desc, t) {
                    Ok(d) => {
                        self.dyns.insert((id, i), d);
                    }
                    Err(e) => self.note(Level::Error, "text.dynamic", scene, id, e),
                }
            }
        }
        for id in enter {
            self.enter(id, t);
        }
        for id in leave {
            self.leave(id, t);
        }
        let runs_before = self.runs.len();
        self.build_runs(scene);
        let added = self.enter_added(scene, &known, &fresh, &visible, t);
        for i in runs_before..self.runs.len() {
            if added.contains(&self.runs[i].root) {
                self.run_play(i, t);
            }
        }
        self.initialized = true;
    }

    /// Узлы, которые данные добавили в уже играющую сцену (элементы `For`), входят сразу: своё
    /// `in`, а у элемента без своего `in` — каскад `stagger` контейнера, по порядку среди
    /// добавленных вместе. Новый корень под stage (экземпляр канала) ждёт своего play.
    fn enter_added(&mut self, scene: &Scene, known: &BTreeSet<u32>, fresh: &[u32], visible: &BTreeMap<u32, bool>, t: f64) -> BTreeSet<u32> {
        let mut entered = BTreeSet::new();
        if self.scene != ScenePhase::Playing || fresh.is_empty() {
            return entered;
        }
        let stage = scene.stage.as_ref().map(|s| s.id);
        let top = |mut id: u32| {
            while let Some(p) = scene.node(id).parent.filter(|p| Some(*p) != stage) {
                id = p;
            }
            id
        };
        let added: Vec<u32> = fresh
            .iter()
            .copied()
            .filter(|&id| {
                let root = top(id);
                known.contains(&root) && !self.pending.contains(&root) && visible.get(&id).copied().unwrap_or(false)
            })
            .collect();
        for &id in &added {
            self.enter(id, t);
            entered.insert(id);
        }
        let mut by_parent: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for &id in &added {
            if let Some(p) = scene.node(id).parent.filter(|p| known.contains(p)) {
                by_parent.entry(p).or_default().push(id);
            }
        }
        for (p, kids) in by_parent {
            let Some(enter) = self.nodes.get(&p).and_then(|s| s.presence.as_ref()).and_then(|pr| pr.enter.clone()) else { continue };
            let own_in = |s: &NodeState| s.presence.as_ref().is_some_and(|pr| pr.enter.is_some());
            let order: Vec<u32> =
                scene.node(p).children.iter().copied().filter(|c| kids.contains(c) && !self.nodes.get(c).is_some_and(own_in)).collect();
            for part in &enter.per_child {
                let mut order = order.clone();
                if part.reverse {
                    order.reverse();
                }
                for (i, c) in order.into_iter().enumerate() {
                    let shifted = part.program.clone().shifted(part.offset + part.step * i as f64);
                    let end = t + shifted.duration;
                    self.nodes.get_mut(&c).expect("node state").start(&shifted, t, "in", false);
                    self.settled_at = Some(self.settled_at.map_or(end, |s| s.max(end)));
                    self.horizon = self.horizon.max(end);
                }
            }
        }
        entered
    }

    /// Узлы, которые данные убрали (элементы `For`, экземпляры канала после stop): уходят через
    /// свой `out` (и таймлайн — на маркер `out`), а элемент без своего `out` — по каскаду `out`
    /// контейнера. Когда уход кончился, `hide_time` корня говорит, что его можно удалить.
    pub fn release(&mut self, scene: &Scene, roots: &[u32], t: f64) {
        for &r in roots {
            self.stop(scene, Some(&[r]), t);
        }
        let mut by_parent: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for &r in roots {
            if let Some(p) = scene.node(r).parent.filter(|p| !roots.contains(p)) {
                by_parent.entry(p).or_default().push(r);
            }
        }
        for (p, kids) in by_parent {
            let Some(exit) = self.nodes.get(&p).and_then(|s| s.presence.as_ref()).and_then(|pr| pr.exit.clone()) else { continue };
            let own_out = |s: &NodeState| s.presence.as_ref().is_some_and(|pr| pr.exit.is_some());
            let order: Vec<u32> = scene
                .node(p)
                .children
                .iter()
                .copied()
                .filter(|c| kids.contains(c) && self.nodes.get(c).is_some_and(|s| s.visible && !own_out(s)))
                .collect();
            for part in &exit.per_child {
                let mut order = order.clone();
                if part.reverse {
                    order.reverse();
                }
                for (i, c) in order.into_iter().enumerate() {
                    let shifted = part.program.clone().shifted(part.offset + part.step * i as f64);
                    let end = t + shifted.duration;
                    let st = self.nodes.get_mut(&c).expect("node state");
                    st.start(&shifted, t, "out", true);
                    st.phase = Phase::Out { start: t, end };
                    let hide = st.hide_at.map_or(end, |h| h.max(end));
                    self.schedule(end, Event::OutEnd(c));
                    self.hide(c, hide);
                }
            }
        }
    }

    /// Воспроизведения таймлайнов для экземпляров, у которых их ещё нет: при монтировании
    /// сцены — для всех, на канале — для каждого нового экземпляра.
    fn build_runs(&mut self, scene: &Scene) {
        if !self.initialized {
            self.diagnostics.0.extend(scene.timeline_diagnostics.0.iter().cloned());
        }
        for def in &scene.timelines {
            let roots: Vec<(u32, String)> = scene
                .nodes
                .values()
                .filter_map(|n| n.instance().filter(|i| component_of(i) == def.component).map(|i| (n.id, i.to_string())))
                .filter(|(id, _)| !self.with_runs.contains(id))
                .collect();
            if roots.is_empty() && !self.initialized && !self.channel {
                self.diagnostics.push(Level::Warn, "timeline.unused", &def.component, None, "в сцене нет экземпляров этого компонента", None);
            }
            for (root, inst) in roots {
                self.with_runs.insert(root);
                let parts: BTreeMap<String, u32> = scene
                    .nodes
                    .values()
                    .filter_map(|n| n.part().filter(|(i, _)| *i == inst).map(|(_, p)| (p.to_string(), n.id)))
                    .collect();
                for k in &def.keys {
                    if !parts.contains_key(&k.part) {
                        let msg = format!("ключ на {}s адресует часть {:?}, а в view нет узла с name={{part.{}}}", num(k.t as f32), k.part, k.part);
                        self.note(Level::Error, "timeline.part", scene, root, msg);
                    }
                }
                let nodes = &self.nodes;
                let start = |id: u32, ch: Channel| match ch.identity() {
                    Some(v) => V::N(v),
                    None => nodes.get(&id).map_or(V::N(0.0), |st| st.abs_value(ch, 0.0)),
                };
                let run = Run::new(inst, root, def.clone(), &parts, start);
                self.runs.push(run);
            }
        }
    }

    fn schedule_run(&mut self, i: usize) {
        let run = &mut self.runs[i];
        run.generation += 1;
        let generation = run.generation;
        if let Some((at, _)) = run.next_stop() {
            self.schedule(at, Event::Timeline(i, generation));
        }
        self.drain(i);
    }

    /// Переносит события воспроизведения в общий журнал по времени.
    fn drain(&mut self, i: usize) {
        let events = std::mem::take(&mut self.runs[i].events);
        let source = self.runs[i].inst.clone();
        for e in events {
            self.push_event(MotionEvent { t: e.t, source: source.clone(), kind: e.kind, name: e.name });
        }
    }

    /// Событие в журнал по времени. Канал пишет сюда жизненный цикл экземпляров на слоях.
    pub fn push_event(&mut self, e: MotionEvent) {
        let pos = self.events_log.partition_point(|x| x.t <= e.t);
        self.events_log.insert(pos, e);
    }

    /// Режим канала: сцена играет с момента `t`, экземпляры появляются по командам.
    pub fn start_channel(&mut self, t: f64) {
        self.channel = true;
        self.scene = ScenePhase::Playing;
        self.scene_event(t, "playing");
    }

    /// Экземпляр загружен на слой: не виден, пока его не запустит play.
    /// Когда закончится `in` экземпляра, начатый в `t`: узлы с presence и таймлайн до первой
    /// остановки. Для расписания перехода (по копии состояния, см. `Session`).
    pub fn in_end(&self, scene: &Scene, root: u32, t: f64) -> f64 {
        let mut end = t;
        for id in Self::subtree(scene, root) {
            if let Some(NodeState { phase: Phase::In { end: e, .. }, .. }) = self.nodes.get(&id)
                && e.is_finite()
            {
                end = end.max(*e);
            }
        }
        for i in self.runs_of(&[root]) {
            end = end.max(t + self.runs[i].def.settle_tau());
        }
        end
    }

    /// Узел уходит из кадра в момент `at` (снятие экземпляра).
    fn hide(&mut self, id: u32, at: f64) {
        let st = self.nodes.get_mut(&id).expect("node state");
        st.hide_at = Some(at);
        st.stopped = false;
        self.schedule(at, Event::Hide(id));
    }

    pub fn hold_loaded(&mut self, root: u32) {
        self.pending.insert(root);
    }

    /// Когда экземпляр, которому дали stop, уйдёт с экрана целиком.
    pub fn hide_time(&self, root: u32) -> Option<f64> {
        self.nodes.get(&root)?.hide_at
    }

    /// Забыть удалённые узлы: состояние движения, числа и часы, таймлайны их экземпляров.
    pub fn forget(&mut self, ids: &[u32]) {
        for id in ids {
            self.nodes.remove(id);
            self.pending.remove(id);
            self.with_runs.remove(id);
        }
        self.dyns.retain(|(id, _), _| !ids.contains(id));
        for run in &mut self.runs {
            if ids.contains(&run.root) {
                run.dead = true;
            }
        }
        self.events.retain(|(_, _, ev)| !matches!(ev, Event::InEnd(id) | Event::OutEnd(id) | Event::Hide(id) if ids.contains(id)));
    }

    fn scene_event(&mut self, t: f64, name: &str) {
        self.push_event(MotionEvent { t, source: "scene".into(), kind: "state", name: name.into() });
    }

    /// Корни экземпляров компонента.
    pub fn roots_of(scene: &Scene, component: &str) -> Vec<u32> {
        scene.nodes.values().filter(|n| n.instance().is_some_and(|i| component_of(i) == component)).map(|n| n.id).collect()
    }

    fn subtree(scene: &Scene, root: u32) -> Vec<u32> {
        let mut out = vec![root];
        let mut i = 0;
        while i < out.len() {
            out.extend(scene.node(out[i]).children.iter().copied());
            i += 1;
        }
        out
    }

    fn compile_presence(&mut self, scene: &Scene, id: u32, p: &Value) -> Presence {
        let mut out = Presence::default();
        let mut c = Compiler { theme: &scene.theme, notes: Vec::new() };
        let mut errors = Vec::new();
        for (key, slot) in [("in", &mut out.enter), ("loop", &mut out.repeat), ("out", &mut out.exit)] {
            let Some(m) = p.get(key) else { continue };
            match c.compile(m) {
                Ok(prog) => *slot = Some(prog),
                Err(e) => errors.push(format!("presence.{key}: {e}")),
            }
        }
        let notes = std::mem::take(&mut c.notes);
        for (level, rule, msg) in notes {
            self.note(level, rule, scene, id, msg);
        }
        for e in errors {
            self.note(Level::Error, "motion.invalid", scene, id, e);
        }
        out
    }

    fn targets_of(&mut self, scene: &Scene, id: u32) -> (BTreeMap<Channel, V>, BTreeMap<String, Easing>) {
        let node = scene.node(id);
        let mut scratch = Diagnostics::default();
        let resolved = look::resolve(scene, node, &[], &mut scratch);
        let visual = look::visual(&resolved, "", None, &mut scratch);
        let mut targets = BTreeMap::new();
        targets.insert(Channel::Opacity, V::N(visual.opacity as f64));
        let tr = visual.transform;
        targets.insert(Channel::X, V::N(tr.as_ref().map_or(0.0, |t| t.x) as f64));
        targets.insert(Channel::Y, V::N(tr.as_ref().map_or(0.0, |t| t.y) as f64));
        targets.insert(Channel::Scale, V::N(tr.as_ref().map_or(1.0, |t| t.scale[0]) as f64));
        targets.insert(Channel::Rotate, V::N(tr.as_ref().map_or(0.0, |t| t.rotate) as f64));
        if visual.radius.iter().all(|r| *r == visual.radius[0]) {
            targets.insert(Channel::Radius, V::N(visual.radius[0] as f64));
        }
        targets.insert(Channel::Background, visual.background.map_or(V::C([0.0; 4]), V::from_rgba));
        if node.kind == Kind::Text {
            let text = look::text_look(&resolved, "", None, &mut scratch);
            targets.insert(Channel::Color, V::from_rgba(text.color));
        }
        if let Some(size) = node.props.get("size") {
            if let Some(w) = size.get("w").and_then(Value::as_f64) {
                targets.insert(Channel::Width, V::N(w));
            }
            if let Some(h) = size.get("h").and_then(Value::as_f64) {
                targets.insert(Channel::Height, V::N(h));
            }
        }

        let mut transitions = BTreeMap::new();
        if let Some(map) = resolved.get("transition").and_then(Value::as_object) {
            for (key, e) in map {
                if key != "all" && key != "layout" && Channel::parse(key).is_none() {
                    self.note(Level::Warn, "motion.unsupported", scene, id, format!("transition.{key}: канал не анимируется"));
                    continue;
                }
                match Easing::parse(e) {
                    Ok(e) => {
                        transitions.insert(key.clone(), e);
                    }
                    Err(err) => self.note(Level::Error, "motion.invalid", scene, id, format!("transition.{key}: {err}")),
                }
            }
        }
        (targets, transitions)
    }

    /// Запускает программу на узле и её каскад (`stagger`) на детях; возвращает момент окончания.
    fn run_program(&mut self, id: u32, prog: &Program, t: f64, source: &'static str, continuing: bool) -> f64 {
        let st = self.nodes.get_mut(&id).expect("node state");
        st.start(prog, t, source, continuing);
        let children = st.children.clone();
        let mut end = if prog.infinite { f64::INFINITY } else { t + prog.duration };
        for part in &prog.per_child {
            let mut kids: Vec<u32> = children.iter().copied().filter(|c| self.nodes.get(c).is_some_and(|s| s.visible)).collect();
            if part.reverse {
                kids.reverse();
            }
            for (i, c) in kids.into_iter().enumerate() {
                let shifted = part.program.clone().shifted(part.offset + part.step * i as f64);
                end = end.max(if shifted.infinite { f64::INFINITY } else { t + shifted.duration });
                let kid = self.nodes.get_mut(&c).expect("node state");
                let on_screen = continuing && !matches!(kid.phase, Phase::Waiting | Phase::Gone);
                kid.start(&shifted, t, source, on_screen);
            }
        }
        end
    }

    fn enter(&mut self, id: u32, t: f64) {
        let st = self.nodes.get_mut(&id).expect("node state");
        let Some(p) = st.presence.clone() else {
            st.phase = Phase::Shown;
            return;
        };
        let continuing = !matches!(st.phase, Phase::Waiting | Phase::Gone);
        if let Some(enter) = &p.enter {
            let end = self.run_program(id, enter, t, "in", continuing);
            let st = self.nodes.get_mut(&id).expect("node state");
            st.phase = Phase::In { start: t, end };
            if end.is_finite() {
                self.settled_at = Some(self.settled_at.map_or(end, |s| s.max(end)));
                self.schedule(end, Event::InEnd(id));
            }
        } else if let Some(repeat) = &p.repeat {
            self.run_program(id, repeat, t, "loop", continuing);
            self.nodes.get_mut(&id).expect("node state").phase = Phase::Loop;
        } else {
            st.phase = Phase::Shown;
        }
    }

    /// Запускает `out`; возвращает, когда узел уйдёт.
    fn leave(&mut self, id: u32, t: f64) -> f64 {
        let st = self.nodes.get_mut(&id).expect("node state");
        match st.presence.as_ref().and_then(|p| p.exit.clone()) {
            Some(exit) => {
                let end = self.run_program(id, &exit, t, "out", true);
                let end = if end.is_finite() { end } else { t + exit.duration };
                self.nodes.get_mut(&id).expect("node state").phase = Phase::Out { start: t, end };
                self.schedule(end, Event::OutEnd(id));
                end
            }
            None => {
                st.phase = Phase::Gone;
                t
            }
        }
    }

    fn run_play(&mut self, i: usize, t: f64) {
        self.runs[i].play(t);
        let settled = t + self.runs[i].def.settle_tau();
        self.settled_at = Some(self.settled_at.map_or(settled, |s| s.max(settled)));
        self.schedule_run(i);
    }

    fn runs_of(&self, roots: &[u32]) -> Vec<usize> {
        (0..self.runs.len()).filter(|i| !self.runs[*i].dead && roots.contains(&self.runs[*i].root)).collect()
    }

    /// Воспроизведения живых экземпляров.
    fn live_runs(&self) -> Vec<usize> {
        (0..self.runs.len()).filter(|i| !self.runs[*i].dead).collect()
    }

    /// play: узлы с presence начинают `in` (или сразу `loop`), таймлайны — воспроизведение.
    /// `roots` — только эти экземпляры; `None` — вся сцена.
    pub fn play(&mut self, scene: &Scene, roots: Option<&[u32]>, t: f64) {
        let Some(roots) = roots else {
            if self.scene == ScenePhase::Playing {
                self.push_event(MotionEvent { t, source: "scene".into(), kind: "ignored", name: "play: уже идёт".into() });
                return;
            }
            self.scene = ScenePhase::Playing;
            self.scene_event(t, "playing");
            self.settled_at = Some(t);
            let ids: Vec<u32> = self.nodes.iter().filter(|(_, s)| s.presence.is_some() && s.visible).map(|(id, _)| *id).collect();
            for id in ids {
                self.enter(id, t);
            }
            for i in self.live_runs() {
                self.run_play(i, t);
            }
            return;
        };
        if matches!(self.scene, ScenePhase::Loaded | ScenePhase::Removed) {
            self.scene = ScenePhase::Playing;
            self.scene_event(t, "playing");
        }
        self.settled_at.get_or_insert(t);
        for root in roots {
            self.pending.remove(root);
        }
        for &root in roots {
            for id in Self::subtree(scene, root) {
                let Some(st) = self.nodes.get_mut(&id) else { continue };
                st.hide_at = None;
                st.stopped = false;
                let fresh = matches!(st.phase, Phase::Waiting | Phase::Gone);
                if fresh && st.visible {
                    self.enter(id, t);
                }
            }
        }
        for i in self.runs_of(roots) {
            self.run_play(i, t);
        }
    }

    /// play «сразу в эфире»: `in` уже закончился к моменту `t`, таймлайн стоит там, где
    /// воспроизведение останавливается первым (hold или цикл). Для подъёма после сбоя и preview:
    /// зритель не должен видеть повторный выход графики. События in и маркеров — в прошлом.
    pub fn play_settled(&mut self, scene: &Scene, roots: &[u32], t: f64) {
        let saved = self.settled_at;
        for root in roots {
            self.pending.remove(root);
        }
        for &root in roots {
            for id in Self::subtree(scene, root) {
                let Some(st) = self.nodes.get_mut(&id) else { continue };
                st.hide_at = None;
                st.stopped = false;
                if !(matches!(st.phase, Phase::Waiting | Phase::Gone) && st.visible) {
                    continue;
                }
                self.enter(id, t);
                // Длительность in известна, когда он запущен: тот же in, но начатый раньше на неё.
                if let Some(Phase::In { start, end }) = self.nodes.get(&id).map(|s| s.phase)
                    && end.is_finite()
                {
                    self.nodes.get_mut(&id).expect("node state").phase = Phase::Waiting;
                    self.enter(id, t - (end - start));
                }
            }
        }
        for i in self.runs_of(roots) {
            let tau = self.runs[i].def.settle_tau();
            self.run_play(i, t - tau);
        }
        self.settled_at = Some(saved.map_or(t, |s| s.max(t)));
    }

    /// Уходящий экземпляр снова нужен: узлы посреди `out` играют `in` с того места, где они
    /// сейчас (как любое прерывание), уже ушедшие — `in` с объявленных значений; снятие
    /// отменяется, таймлайн играет заново.
    pub fn revive(&mut self, scene: &Scene, roots: &[u32], t: f64) {
        self.settled_at.get_or_insert(t);
        for &root in roots {
            for id in Self::subtree(scene, root) {
                let Some(st) = self.nodes.get_mut(&id) else { continue };
                st.hide_at = None;
                st.stopped = false;
                let back = matches!(st.phase, Phase::Out { .. }) || (st.phase == Phase::Gone && st.visible);
                if back {
                    self.enter(id, t);
                }
            }
        }
        for i in self.runs_of(roots) {
            self.run_play(i, t);
        }
    }

    /// stop: узлы с `out` начинают его, таймлайны прыгают на маркер `out`; сцена (или экземпляр)
    /// снимается, когда закончится последний. Узлы без `out` остаются на экране до снятия.
    pub fn stop(&mut self, scene: &Scene, roots: Option<&[u32]>, t: f64) {
        let ids: Vec<u32> = match roots {
            None => self.nodes.keys().copied().collect(),
            Some(roots) => roots.iter().flat_map(|r| Self::subtree(scene, *r)).collect(),
        };
        if roots.is_none() {
            match self.scene {
                ScenePhase::Playing => {}
                // Второй stop не обрывает идущий уход.
                ScenePhase::Stopping { .. } | ScenePhase::Removed => {
                    self.push_event(MotionEvent { t, source: "scene".into(), kind: "ignored", name: "stop: сцена уже уходит".into() });
                    return;
                }
                ScenePhase::Loaded => {
                    self.scene = ScenePhase::Removed;
                    self.scene_event(t, "removed");
                    return;
                }
            }
        }
        let leaving: Vec<u32> = ids
            .into_iter()
            .filter(|id| {
                self.nodes.get(id).is_some_and(|s| {
                    s.presence.as_ref().is_some_and(|p| p.exit.is_some()) && s.visible && !matches!(s.phase, Phase::Gone | Phase::Out { .. })
                })
            })
            .collect();
        let mut until = t;
        for id in leaving {
            until = until.max(self.leave(id, t));
        }
        let runs = match roots {
            None => self.live_runs(),
            Some(roots) => self.runs_of(roots),
        };
        for i in runs {
            until = until.max(self.runs[i].stop(t));
            self.schedule_run(i);
        }
        match roots {
            None => {
                self.scene = ScenePhase::Stopping { until };
                self.scene_event(t, "stopping");
                self.schedule(until, Event::Removed);
            }
            Some(roots) => {
                // Экземпляр снимается целиком, когда закончился последний out в нём.
                for &root in roots {
                    if self.nodes.contains_key(&root) {
                        self.hide(root, until);
                    }
                }
                self.horizon = self.horizon.max(until);
            }
        }
    }

    /// next: снять остановку таймлайна (hold или цикл).
    pub fn next(&mut self, roots: Option<&[u32]>, t: f64) {
        let runs = match roots {
            None => self.live_runs(),
            Some(roots) => self.runs_of(roots),
        };
        if runs.is_empty() {
            self.push_event(MotionEvent { t, source: "scene".into(), kind: "ignored", name: "next: в сцене нет таймлайнов".into() });
        }
        for i in runs {
            self.runs[i].next(t);
            self.schedule_run(i);
        }
    }

    /// Команда движения от команды компонента (`ctx.play('goal')`, `ctx.stop()`, `ctx.emit(...)`).
    pub fn instance_cmd(&mut self, scene: &Scene, inst: &str, action: &MotionAction, t: f64) -> Result<(), String> {
        let root = scene.nodes.values().find(|n| n.instance() == Some(inst)).map(|n| n.id).ok_or_else(|| format!("экземпляр {inst} не найден"))?;
        let roots = [root];
        match action {
            MotionAction::Play { marker: Some(marker) } => {
                let Some(i) = self.runs_of(&roots).first().copied() else {
                    return Err(format!("ctx.play({marker:?}): у {inst} нет таймлайна; подключи его в defineScene({{ timelines }})"));
                };
                self.runs[i].segment(t, marker)?;
                self.schedule_run(i);
            }
            MotionAction::Play { marker: None } => self.play(scene, Some(&roots), t),
            MotionAction::Stop => self.stop(scene, Some(&roots), t),
            MotionAction::Next => self.next(Some(&roots), t),
            MotionAction::Emit { name } => self.push_event(MotionEvent { t, source: inst.into(), kind: "emit", name: name.clone() }),
        }
        Ok(())
    }

    /// Ближайшее запланированное событие движения.
    pub fn next_event_time(&self) -> Option<f64> {
        self.events.first().map(|e| e.0)
    }

    /// Обрабатывает запланированные события до момента `t` включительно.
    pub fn advance_to(&mut self, t: f64) {
        self.process(|et| et <= t);
        // Маркеры и emit между остановками: их моменты известны точно, журнал дописывается до t.
        for i in self.live_runs() {
            self.runs[i].flush(t);
            self.drain(i);
        }
    }

    /// Обрабатывает события строго до момента `t`: состояние перед событиями в `t`.
    pub fn advance_before(&mut self, t: f64) {
        self.process(|et| et < t);
    }

    fn process(&mut self, due: impl Fn(f64) -> bool) {
        while let Some(&(et, _, ev)) = self.events.first() {
            if !due(et) {
                break;
            }
            self.events.remove(0);
            match ev {
                Event::InEnd(id) => {
                    let st = self.nodes.get_mut(&id).expect("node state");
                    if matches!(st.phase, Phase::In { end, .. } if end == et) {
                        match st.presence.as_ref().and_then(|p| p.repeat.clone()) {
                            Some(repeat) => {
                                self.run_program(id, &repeat, et, "loop", true);
                                self.nodes.get_mut(&id).expect("node state").phase = Phase::Loop;
                            }
                            None => st.phase = Phase::Shown,
                        }
                    }
                }
                Event::OutEnd(id) => {
                    let st = self.nodes.get_mut(&id).expect("node state");
                    if matches!(st.phase, Phase::Out { end, .. } if end == et) {
                        st.phase = Phase::Gone;
                    }
                }
                Event::Hide(id) => {
                    let st = self.nodes.get_mut(&id).expect("node state");
                    if st.hide_at == Some(et) {
                        st.stopped = true;
                    }
                }
                Event::Removed => {
                    if matches!(self.scene, ScenePhase::Stopping { until } if until == et) {
                        self.scene = ScenePhase::Removed;
                        self.scene_event(et, "removed");
                    }
                }
                Event::Timeline(i, generation) => {
                    if !self.runs[i].dead
                        && self.runs[i].generation == generation
                        && let Some((_, stop)) = self.runs[i].next_stop() {
                            self.runs[i].on_stop(et, stop);
                            self.schedule_run(i);
                        }
                }
            }
        }
    }

    /// Есть ли в сцене узлы с `transition.layout`: без них раскладку сверять не нужно.
    pub fn has_layout(&self) -> bool {
        self.nodes.values().any(|s| s.layout.is_some())
    }

    /// Когда закончатся начатые переезды раскладки.
    pub fn layout_end(&self) -> Option<f64> {
        self.nodes.values().flat_map(|s| s.geo.iter().flatten()).filter_map(Track::end).reduce(f64::max)
    }

    /// Ближайшее событие, после которого узел уходит из раскладки (конец `out`, снятие
    /// экземпляра): в этот момент соседи с `transition.layout` переезжают.
    pub fn next_layout_event(&self) -> Option<f64> {
        if !self.has_layout() {
            return None;
        }
        self.events.iter().find(|(_, _, ev)| matches!(ev, Event::OutEnd(_) | Event::Hide(_))).map(|e| e.0)
    }

    /// Раскладка в момент `t` перестроилась: `before` и `after` — геометрия до и после
    /// (без анимации геометрии). Узел с `transition.layout`, у которого изменились место
    /// относительно родителя или размер, едет к новому из того места, где его видно сейчас:
    /// смещение от новой раскладки затухает до нуля, пружина сохраняет скорость.
    pub fn relayout(&mut self, before: &GeoMap, after: &GeoMap, t: f64) {
        for (id, a) in after {
            let Some(b) = before.get(id) else { continue };
            let Some(st) = self.nodes.get_mut(id) else { continue };
            let Some(easing) = st.layout else { continue };
            let axes = if a.sized { 4 } else { 2 };
            for k in 0..axes {
                let change = (b.rel[k] - a.rel[k]) as f64;
                if change.abs() < 1e-3 {
                    continue;
                }
                let track = st.geo[k].take().filter(|tr| tr.running(t));
                let from = track.as_ref().map_or(0.0, |tr| tr.value(t).n()) + change;
                // Перестройки в один момент погасили друг друга: узел уже на месте.
                if from.abs() < 1e-3 {
                    continue;
                }
                let v0 = match (&track, easing) {
                    (Some(tr), Easing::Spring { .. }) if from.abs() > 1e-9 => -tr.velocity(t) / from,
                    _ => 0.0,
                };
                st.geo[k] = Some(Track { segs: vec![Seg::new(t, V::N(from), V::N(0.0), easing, v0, false)], source: "layout" });
            }
        }
    }

    /// Значения всех анимаций в момент `t` (события до `t` уже обработаны).
    pub fn values(&self, t: f64) -> Animated {
        let mut out = Animated { removed: self.scene == ScenePhase::Removed, diagnostics: self.diagnostics.clone(), ..Animated::default() };
        for (&id, st) in &self.nodes {
            let mut nv = NodeValues::default();
            for (&ch, tr) in &st.abs {
                let v = tr.value(t);
                nv.abs.insert(ch, v);
                if tr.running(t) {
                    let progress = progress(tr, t);
                    nv.notes.push(Note::Transition { channel: ch.name(), progress });
                    nv.channels.push(info(ch, "transition", tr, v, t, progress));
                }
            }
            let waiting = st.phase == Phase::Waiting;
            for (&ch, tr) in &st.rel {
                let v = tr.value(t);
                let f = v.n() as f32;
                match ch {
                    Channel::X => nv.motion.dx = f,
                    Channel::Y => nv.motion.dy = f,
                    Channel::Scale => nv.motion.scale = f,
                    Channel::Rotate => nv.motion.rotate = f,
                    Channel::Opacity => nv.motion.opacity = f,
                    Channel::Reveal => nv.motion.reveal = f,
                    _ => {
                        nv.abs.insert(ch, v);
                        if waiting || tr.running(t) {
                            nv.staged.push(ch);
                        }
                    }
                }
                let p = if waiting { Some(0.0) } else { progress(tr, t) };
                nv.channels.push(info(ch, "presence", tr, v, t, p));
            }
            let phase = match st.phase {
                Phase::Waiting => Some(Note::Waiting),
                Phase::In { start, end } => Some(Note::In { progress: span_progress(start, end, t) }),
                Phase::Loop => Some(Note::Loop),
                Phase::Out { start, end } => Some(Note::Out { progress: span_progress(start, end, t) }),
                Phase::Gone => Some(Note::Gone),
                Phase::Shown => None,
            };
            if let Some(p) = phase {
                nv.notes.insert(0, p);
            }
            nv.gone = match st.phase {
                _ if self.pending.contains(&id) => Some(Gone::Loaded),
                _ if st.stopped => Some(Gone::InstanceStopped),
                Phase::Gone => Some(Gone::OutEnded),
                _ => None,
            };
            nv.show = match st.phase {
                _ if nv.gone.is_some() => Some(false),
                Phase::Out { .. } if st.own_hidden => Some(true),
                _ => None,
            };
            let running = st.geo.iter().flatten().filter(|tr| tr.running(t));
            if let Some(last) = running.max_by(|a, b| a.end().unwrap_or(0.0).total_cmp(&b.end().unwrap_or(0.0))) {
                let geo: [f32; 4] = std::array::from_fn(|k| st.geo[k].as_ref().filter(|tr| tr.running(t)).map_or(0.0, |tr| tr.value(t).n() as f32));
                let progress = progress(last, t);
                nv.notes.push(Note::Layout { progress });
                nv.channels.push(ChannelInfo {
                    channel: "layout",
                    layer: "layout",
                    source: "layout",
                    value: format!("Δ {},{} {}×{}", num(geo[0]), num(geo[1]), num(geo[2]), num(geo[3])),
                    easing: last.easing_label(t),
                    progress,
                });
                nv.geo = Some(geo);
            }
            if nv.geo.is_some() || !nv.abs.is_empty() || !nv.motion.is_identity() || nv.show.is_some() || !nv.notes.is_empty() || !nv.channels.is_empty() {
                out.nodes.insert(id, nv);
            }
        }
        for (&(id, i), d) in &self.dyns {
            let nv = out.nodes.entry(id).or_default();
            nv.texts.insert(i, d.text(t));
            if let Dyn::Count { .. } = d
                && d.running(t) {
                    nv.notes.push(Note::Count);
                }
        }
        // Таймлайны поверх presence: сдвиги складываются, множители перемножаются.
        for run in self.runs.iter().filter(|r| !r.dead) {
            for &(id, ch) in run.keys() {
                let Some(v) = run.value((id, ch), t) else { continue };
                let nv = out.nodes.entry(id).or_default();
                let f = v.n() as f32;
                match ch {
                    Channel::X => nv.motion.dx += f,
                    Channel::Y => nv.motion.dy += f,
                    Channel::Scale => nv.motion.scale *= f,
                    Channel::Rotate => nv.motion.rotate += f,
                    Channel::Opacity => nv.motion.opacity *= f,
                    Channel::Reveal => nv.motion.reveal *= f,
                    _ => {
                        nv.abs.insert(ch, v);
                        if run.track((id, ch), t).is_some_and(|(tr, local)| tr.running(local)) {
                            nv.staged.push(ch);
                        }
                    }
                }
                if let Some((tr, local)) = run.track((id, ch), t) {
                    nv.channels.push(ChannelInfo {
                        channel: ch.name(),
                        layer: "timeline",
                        source: run.state.label(),
                        value: v.show(),
                        easing: tr.easing_label(local),
                        progress: progress(tr, local),
                    });
                }
            }
            let root = out.nodes.entry(run.root).or_default();
            root.notes.insert(0, run.describe(t));
            if run.removed() {
                root.show = Some(false);
                root.gone = Some(Gone::TimelineRemoved);
            }
        }
        out
    }
}

fn span_progress(start: f64, end: f64, t: f64) -> Option<f64> {
    (end.is_finite() && end > start).then(|| ((t - start) / (end - start)).clamp(0.0, 1.0))
}

fn progress(tr: &Track, t: f64) -> Option<f64> {
    let start = tr.segs.first()?.start;
    span_progress(start, tr.end()?, t)
}

fn pct(p: Option<f64>) -> String {
    p.map_or("∞".into(), |p| format!("{}", (p * 100.0).round() as i64))
}

fn info(ch: Channel, layer: &'static str, tr: &Track, v: V, t: f64, progress: Option<f64>) -> ChannelInfo {
    ChannelInfo { channel: ch.name(), layer, source: tr.source, value: v.show(), easing: tr.easing_label(t), progress }
}

/// Переход из прозрачного цвета (или в него) меняет только альфу, а не оттенок.
fn fade_colors(from: V, to: V) -> (V, V) {
    match (from, to) {
        (V::C(f), V::C(t)) if f[3] == 0.0 => (V::C([t[0], t[1], t[2], 0.0]), to),
        (V::C(f), V::C(t)) if t[3] == 0.0 => (from, V::C([f[0], f[1], f[2], 0.0])),
        _ => (from, to),
    }
}

/// Имя компонента по имени экземпляра: `Scoreboard#1` → `Scoreboard`.
pub fn component_of(inst: &str) -> &str {
    inst.split('#').next().unwrap_or(inst)
}

/// Узел виден, если ни он, ни его предки не скрыты (`when`).
fn effective_visibility(scene: &Scene) -> BTreeMap<u32, bool> {
    let mut out = BTreeMap::new();
    fn walk(scene: &Scene, id: u32, parent_visible: bool, out: &mut BTreeMap<u32, bool>) {
        let vis = parent_visible && !scene.node(id).hidden();
        out.insert(id, vis);
        for &c in &scene.node(id).children {
            walk(scene, c, vis, out);
        }
    }
    if let Some(stage) = &scene.stage {
        walk(scene, stage.id, true, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bezier_endpoints_and_symmetry() {
        assert_eq!(bezier(EASE_IN_OUT, 0.0), 0.0);
        assert_eq!(bezier(EASE_IN_OUT, 1.0), 1.0);
        assert!((bezier(EASE_IN_OUT, 0.5) - 0.5).abs() < 1e-6);
        assert!(bezier(EASE_IN, 0.5) < 0.5 && bezier(EASE_OUT, 0.5) > 0.5);
        assert!((bezier(LINEAR, 0.3) - 0.3).abs() < 1e-12);
    }

    #[test]
    fn springs_settle_and_overshoot() {
        let bouncy = Easing::Spring { stiffness: 300.0, damping: 12.0, mass: 1.0 };
        let snappy = Easing::Spring { stiffness: 400.0, damping: 30.0, mass: 1.0 };
        let critical = Easing::Spring { stiffness: 100.0, damping: 20.0, mass: 1.0 };
        let over = Easing::Spring { stiffness: 100.0, damping: 40.0, mass: 1.0 };
        for e in [bouncy, snappy, critical, over] {
            let d = e.duration(0.0);
            assert!(d > 0.05 && d < 10.0, "{e:?}: {d}");
            assert!((e.progress(d, 0.0) - 1.0).abs() <= 1e-3, "{e:?}");
            assert_eq!(e.progress(0.0, 0.0), 0.0);
        }
        // Пружина с малым затуханием перелетает цель, критическая — нет.
        let peak = (0..1000).map(|i| bouncy.progress(i as f64 / 1000.0, 0.0)).fold(0.0, f64::max);
        assert!(peak > 1.2, "{peak}");
        let peak = (0..2000).map(|i| critical.progress(i as f64 / 1000.0, 0.0)).fold(0.0, f64::max);
        assert!(peak <= 1.0 + 1e-9, "{peak}");
    }

    #[test]
    fn spring_keeps_initial_velocity() {
        let s = Easing::Spring { stiffness: 400.0, damping: 30.0, mass: 1.0 };
        let h = 1e-6;
        let v = (s.progress(h, 3.0) - s.progress(0.0, 3.0)) / h;
        assert!((v - 3.0).abs() < 1e-3, "{v}");
    }

    fn program(v: Value) -> Program {
        let theme = BTreeMap::new();
        Compiler { theme: &theme, notes: Vec::new() }.compile(&v).unwrap()
    }

    fn tween(d: f64) -> Value {
        json!({ "kind": "tween", "duration": d, "ease": "linear" })
    }

    #[test]
    fn seq_par_delay_offsets() {
        let a = json!({ "motion": "animate", "targets": { "opacity": [0, 1] }, "easing": tween(0.2) });
        let b = json!({ "motion": "animate", "targets": { "x": [-40, 0] }, "easing": tween(0.5) });
        let p = program(json!({ "motion": "seq", "items": [a, { "motion": "delay", "seconds": 0.1, "item": b }] }));
        assert_eq!(p.segs.len(), 2);
        assert!((p.segs[1].offset - 0.3).abs() < 1e-12);
        assert!((p.duration - 0.8).abs() < 1e-12);
        let p = program(json!([a, b]));
        assert!((p.duration - 0.5).abs() < 1e-12 && p.segs.iter().all(|s| s.offset == 0.0));
    }

    #[test]
    fn interrupted_motion_starts_from_current_value() {
        let fade_in = program(json!({ "motion": "animate", "targets": { "opacity": [0, 1] }, "easing": tween(1.0) }));
        let fade_out = program(json!({ "motion": "animate", "targets": { "opacity": [1, 0] }, "easing": tween(1.0) }));
        let mut st = NodeState::default();
        st.start(&fade_in, 0.0, "in", false);
        assert!((st.current(Channel::Opacity, 0.4).n() - 0.4).abs() < 1e-9);
        // out посреди in: идёт от 0.4, а не от объявленной 1.
        st.start(&fade_out, 0.4, "out", true);
        assert!((st.current(Channel::Opacity, 0.4).n() - 0.4).abs() < 1e-9);
        assert!((st.current(Channel::Opacity, 0.9).n() - 0.2).abs() < 1e-9);
        assert_eq!(st.current(Channel::Opacity, 2.0).n(), 0.0);
    }

    #[test]
    fn pulse_swings_between_values() {
        let p = program(json!({ "motion": "pulse", "period": 1.0, "opacity": [1, 0.4] }));
        assert!(p.infinite);
        let mut st = NodeState::default();
        st.start(&p, 0.0, "loop", false);
        assert!((st.current(Channel::Opacity, 0.0).n() - 1.0).abs() < 1e-9);
        assert!((st.current(Channel::Opacity, 0.5).n() - 0.4).abs() < 1e-9);
        assert!((st.current(Channel::Opacity, 1.0).n() - 1.0).abs() < 1e-9);
        assert!((st.current(Channel::Opacity, 10.5).n() - 0.4).abs() < 1e-9);
    }
}
