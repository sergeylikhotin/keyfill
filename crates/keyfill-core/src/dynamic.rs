//! Куски текста, которые ведёт ядро: `count()` — анимированное число, `clock()` — часы и
//! таймеры. JS сообщает только смену данных; значение на каждом кадре считается здесь,
//! поэтому кадр вычислим в любой момент `t`.

use serde_json::Value;

use crate::motion::{Easing, Seg, Track, V};

#[derive(Debug, Clone)]
pub enum Dyn {
    Count { target: f64, track: Option<Track>, format: String },
    Clock(ClockState),
}

#[derive(Debug, Clone)]
pub struct ClockState {
    /// Последнее `from` из JS: его смена сбрасывает часы.
    base: f64,
    /// Значение в момент `t0`.
    v0: f64,
    t0: f64,
    running: bool,
    rate: f64,
    min: Option<f64>,
    max: Option<f64>,
    format: String,
}

impl ClockState {
    pub fn value(&self, t: f64) -> f64 {
        let v = self.v0 + if self.running { self.rate * (t - self.t0) } else { 0.0 };
        let v = self.min.map_or(v, |m| v.max(m));
        self.max.map_or(v, |m| v.min(m))
    }
}

impl Dyn {
    /// Новое описание из JS в момент `t`. Возвращает ошибку разбора для диагностики.
    pub fn update(prev: Option<&Dyn>, desc: &Value, t: f64) -> Result<Dyn, String> {
        let num = |k: &str| desc.get(k).and_then(Value::as_f64);
        let format = desc.get("format").and_then(Value::as_str).unwrap_or("0").to_string();
        match desc.get("kind").and_then(Value::as_str) {
            Some("count") => {
                let target = num("value").ok_or("count: значение должно быть числом")?;
                let easing = desc.get("easing").map(Easing::parse).transpose()?;
                let track = match (prev, easing) {
                    (Some(Dyn::Count { target: old, track, .. }), Some(e)) if *old != target => {
                        let current = track.as_ref().map_or(*old, |tr| tr.value(t).n());
                        Some(Track { segs: vec![Seg::new(t, V::N(current), V::N(target), e, 0.0, false)], source: "count" })
                    }
                    (Some(Dyn::Count { target: old, track, .. }), _) if *old == target => track.clone(),
                    _ => None,
                };
                Ok(Dyn::Count { target, track, format })
            }
            Some("clock") => {
                let from = num("from").ok_or("clock: from должно быть числом секунд")?;
                let running = desc.get("running").and_then(Value::as_bool).unwrap_or(true);
                let rate = num("rate").unwrap_or(1.0);
                let (min, max) = (num("min"), num("max"));
                let format = desc.get("format").and_then(Value::as_str).unwrap_or("mm:ss").to_string();
                check_clock_format(&format)?;
                let v0 = match prev {
                    Some(Dyn::Clock(c)) if c.base == from => c.value(t),
                    _ => from,
                };
                Ok(Dyn::Clock(ClockState { base: from, v0, t0: t, running, rate, min, max, format }))
            }
            other => Err(format!("неизвестный динамический кусок текста {other:?}")),
        }
    }

    pub fn text(&self, t: f64) -> String {
        match self {
            Dyn::Count { target, track, format } => {
                let v = track.as_ref().map_or(*target, |tr| tr.value(t).n());
                format_number(v, format)
            }
            Dyn::Clock(c) => format_clock(c.value(t), &c.format, c.rate < 0.0),
        }
    }

    /// Идёт ли сейчас анимация (для пометок в дампе).
    pub fn running(&self, t: f64) -> bool {
        match self {
            Dyn::Count { track, .. } => track.as_ref().is_some_and(|tr| tr.end().is_some_and(|e| t < e)),
            Dyn::Clock(c) => c.running,
        }
    }
}

/// `'0'` — целое, `'0.0'` — один знак, `'00'` — минимум две цифры в целой части.
pub fn format_number(v: f64, format: &str) -> String {
    let (int, frac) = format.split_once('.').unwrap_or((format, ""));
    let decimals = frac.chars().filter(|c| *c == '0').count();
    let width = int.chars().filter(|c| *c == '0').count().max(1);
    let scale = 10f64.powi(decimals as i32);
    let r = (v * scale).round() / scale;
    let neg = r < 0.0;
    let abs = r.abs();
    let mut s = format!("{abs:.decimals$}");
    let int_len = s.find('.').unwrap_or(s.len());
    if int_len < width {
        s = "0".repeat(width - int_len) + &s;
    }
    if neg { format!("-{s}") } else { s }
}

fn check_clock_format(format: &str) -> Result<(), String> {
    let ok = format.split(':').enumerate().all(|(i, part)| {
        let unit = part.split('.').next().unwrap_or("");
        let frac_ok = part.split_once('.').is_none_or(|(_, f)| !f.is_empty() && f.chars().all(|c| c == '0'));
        let unit_ok = matches!(unit, "h" | "hh" | "m" | "mm" | "s" | "ss");
        unit_ok && frac_ok && (i == format.split(':').count() - 1 || !part.contains('.'))
    });
    if ok { Ok(()) } else { Err(format!("clock: формат {format:?} не разобран; примеры: mm:ss, m:ss, h:mm:ss, ss.0")) }
}

/// Часы: старшая единица не ограничена (`m:ss` при 75 минутах — `75:00`).
/// Обратный отсчёт округляется вверх, чтобы 0:00 появлялось ровно в конце.
pub fn format_clock(seconds: f64, format: &str, countdown: bool) -> String {
    let parts: Vec<&str> = format.split(':').collect();
    let last = parts.last().copied().unwrap_or("ss");
    let decimals = last.split_once('.').map_or(0, |(_, f)| f.len());
    let scale = 10f64.powi(decimals as i32);
    let neg = seconds < 0.0;
    let ticks = if countdown { (seconds.abs() * scale - 1e-9).ceil() } else { (seconds.abs() * scale + 1e-9).floor() };
    let total = ticks / scale;
    let whole = total.trunc() as u64;
    let frac = total - whole as f64;

    let units: Vec<char> = parts.iter().map(|p| p.chars().next().unwrap_or('s')).collect();
    let mut out = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        let unit = units[i];
        let size = match unit {
            'h' => 3600,
            'm' => 60,
            _ => 1,
        };
        let value = if i == 0 { whole / size } else { (whole % if unit == 'm' { 3600 } else { 60 }) / size };
        let width = part.split('.').next().unwrap_or("").len();
        let mut s = format!("{value:0width$}");
        if i == parts.len() - 1 && decimals > 0 {
            let f = format!("{frac:.decimals$}");
            s.push_str(&f[1..]);
        }
        out.push(s);
    }
    let s = out.join(":");
    if neg { format!("-{s}") } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn numbers() {
        assert_eq!(format_number(2.6, "0"), "3");
        assert_eq!(format_number(2.25, "0.0"), "2.3");
        assert_eq!(format_number(7.0, "00"), "07");
        assert_eq!(format_number(-1.5, "0.0"), "-1.5");
    }

    #[test]
    fn clocks() {
        assert_eq!(format_clock(67.0 * 60.0 + 5.4, "mm:ss", false), "67:05");
        assert_eq!(format_clock(3725.0, "h:mm:ss", false), "1:02:05");
        assert_eq!(format_clock(9.87, "ss.0", false), "09.8");
        // Обратный отсчёт: 0.2 с до конца — ещё «1».
        assert_eq!(format_clock(0.2, "m:ss", true), "0:01");
        assert_eq!(format_clock(0.0, "m:ss", true), "0:00");
        assert!(check_clock_format("mm:ss").is_ok() && check_clock_format("ss.0").is_ok());
        assert!(check_clock_format("mm:xx").is_err() && check_clock_format("mm.0:ss").is_err());
    }

    #[test]
    fn clock_pauses_and_resets() {
        let desc = |from: f64, running: bool| json!({ "kind": "clock", "from": from, "running": running, "format": "m:ss" });
        let c = Dyn::update(None, &desc(60.0, true), 0.0).unwrap();
        assert_eq!(c.text(5.0), "1:05");
        // Пауза в 5 с: значение замирает на 65.
        let c = Dyn::update(Some(&c), &desc(60.0, false), 5.0).unwrap();
        assert_eq!(c.text(20.0), "1:05");
        // Пуск продолжает с 65, а смена from сбрасывает.
        let c = Dyn::update(Some(&c), &desc(60.0, true), 20.0).unwrap();
        assert_eq!(c.text(22.0), "1:07");
        let c = Dyn::update(Some(&c), &desc(0.0, true), 22.0).unwrap();
        assert_eq!(c.text(23.0), "0:01");
    }

    #[test]
    fn count_animates_to_new_value() {
        let desc = |v: f64| json!({ "kind": "count", "value": v, "easing": { "kind": "tween", "duration": 1.0, "ease": "linear" }, "format": "0" });
        let c = Dyn::update(None, &desc(2.0), 0.0).unwrap();
        assert_eq!(c.text(0.0), "2");
        let c = Dyn::update(Some(&c), &desc(12.0), 1.0).unwrap();
        assert_eq!(c.text(1.5), "7");
        assert!(c.running(1.5) && !c.running(2.5));
        assert_eq!(c.text(3.0), "12");
    }
}
