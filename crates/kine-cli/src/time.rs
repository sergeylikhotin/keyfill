//! Грамматика времени — одна для `--t`, `--cue` и (позже) MCP:
//!
//! - `1.2` — секунды, `60f` — кадры;
//! - `settled` — кадр «в эфире», когда закончились все `in` первого play;
//! - `out`, `out+0.2`, `stats-5f` — маркер таймлайна со смещением: момент, когда воспроизведение
//!   впервые проходит маркер;
//! - `0..4`, `settled..out+0.5` — диапазон (`check`, `filmstrip`, `video`);
//! - там, где ждут диапазон, имя без смещения — сегмент: `in` (от первого play до settled),
//!   маркер таймлайна (до следующего маркера), `out` без такого маркера (от первого stop до конца).
//!
//! Разбор отделён от вычисления: момент `settled` и маркеры известны только сессии.

use anyhow::{Context as _, Result, bail};

use crate::Session;

/// Длительность: секунды или кадры.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Offset {
    Seconds(f64),
    Frames(i64),
}

impl Offset {
    fn seconds(self, fps: f64) -> f64 {
        match self {
            Offset::Seconds(s) => s,
            Offset::Frames(f) => f as f64 / fps,
        }
    }
}

/// Один момент.
#[derive(Debug, Clone, PartialEq)]
pub enum Moment {
    At(Offset),
    Settled(Offset),
    Marker(String, Offset),
}

/// Момент или диапазон.
#[derive(Debug, Clone, PartialEq)]
pub enum TimeSpec {
    Moment(Moment),
    Range(Moment, Moment),
}

fn offset(s: &str) -> Result<Offset> {
    if let Some(f) = s.strip_suffix('f') {
        let n: i64 = f.parse().map_err(|_| anyhow::anyhow!("{s:?}: ожидается число кадров, например 60f"))?;
        return Ok(Offset::Frames(n));
    }
    let v: f64 = s.parse().map_err(|_| anyhow::anyhow!("{s:?}: ожидаются секунды (1.2) или кадры (60f)"))?;
    if !v.is_finite() {
        bail!("{s:?}: время должно быть конечным");
    }
    Ok(Offset::Seconds(v))
}

pub fn parse_moment(s: &str) -> Result<Moment> {
    let s = s.trim();
    if s.is_empty() {
        bail!("пустое время");
    }
    if s.starts_with(|c: char| c.is_ascii_digit() || c == '.') {
        let o = offset(s)?;
        if matches!(o, Offset::Seconds(v) if v < 0.0) || matches!(o, Offset::Frames(f) if f < 0) {
            bail!("{s:?}: время не может быть отрицательным");
        }
        return Ok(Moment::At(o));
    }
    // Имя с необязательным смещением: settled, out+0.2, stats-5f.
    let split = s.find(['+', '-']);
    let (name, shift) = match split {
        Some(i) => {
            let sign = if &s[i..i + 1] == "-" { -1.0 } else { 1.0 };
            let o = match offset(&s[i + 1..])? {
                Offset::Seconds(v) => Offset::Seconds(sign * v),
                Offset::Frames(f) => Offset::Frames(sign as i64 * f),
            };
            (&s[..i], o)
        }
        None => (s, Offset::Seconds(0.0)),
    };
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        bail!("{s:?}: ожидается секунды (1.2), кадры (60f), settled или маркер со смещением (out+0.2)");
    }
    Ok(match name {
        "settled" => Moment::Settled(shift),
        _ => Moment::Marker(name.to_string(), shift),
    })
}

pub fn parse(s: &str) -> Result<TimeSpec> {
    match s.split_once("..") {
        Some((a, b)) => Ok(TimeSpec::Range(parse_moment(a)?, parse_moment(b)?)),
        None => Ok(TimeSpec::Moment(parse_moment(s)?)),
    }
}

/// Момент в секундах. `settled` и маркеры продвигают сессию вперёд, поэтому моменты
/// разрешаются на отдельной сессии, а кадры рендерит новая.
pub fn resolve(m: &Moment, session: &mut Session) -> Result<f64> {
    let fps = session.fps;
    let t = match m {
        Moment::At(o) => o.seconds(fps),
        Moment::Settled(o) => session.settled_time()? + o.seconds(fps),
        Moment::Marker(name, o) => session.marker_time(name)? + o.seconds(fps),
    };
    if t < 0.0 {
        bail!("момент {t:.3} с раньше начала сцены");
    }
    Ok(t)
}

/// Сегмент по имени: `in`, маркер таймлайна или `out`.
pub fn segment(name: &str, session: &mut Session) -> Result<(f64, f64)> {
    let markers: Vec<Vec<(String, f64)>> = session.scene.timelines.iter().map(crate::ops::timeline_markers).collect();
    // Следующий маркер того же таймлайна и длина сегмента по времени таймлайна.
    let next = markers.iter().find_map(|ms| {
        let i = ms.iter().position(|(n, _)| n == name)?;
        Some(ms.get(i + 1).map(|(n, tau)| (n.clone(), tau - ms[i].1)))
    });
    if let Some(next) = next {
        let start = session.marker_time(name)?;
        let end = match next {
            // Конец — момент, когда воспроизведение проходит следующий маркер. Сегмент команды
            // (`ctx.play(marker)`) до него не доходит, а возвращается назад: тогда берётся длина
            // сегмента по таймлайну.
            Some((n, len)) => session.marker_time(&n).unwrap_or(start + len),
            None => session.end_time()?,
        };
        return Ok((start, end));
    }
    let first = |cmd: &str| session.cues().iter().find(|c| c.cmd == cmd).map(|c| c.t);
    match name {
        "in" => {
            let play = first("play").unwrap_or(0.0);
            Ok((play, session.settled_time()?))
        }
        "out" => {
            let stop = first("stop").context("сегмент out: в сценарии нет stop; добавь --cue \"<время> stop\"")?;
            Ok((stop, session.end_time()?))
        }
        _ => {
            let mut known = vec!["in".to_string(), "out".to_string()];
            known.extend(markers.iter().flatten().map(|(n, _)| n.clone()));
            let mut seen = std::collections::HashSet::new();
            known.retain(|n| seen.insert(n.clone()));
            bail!("нет сегмента {name:?}; есть: {}", known.join(", "))
        }
    }
}

/// Время команды сценария: только абсолютное, секунды или кадры. Кадры переводятся
/// в секунды, когда сессия знает fps.
pub fn cue_time(s: &str) -> Result<Offset> {
    match parse_moment(s)? {
        Moment::At(o) => Ok(o),
        _ => bail!("{s:?}: время команды — секунды (2.5) или кадры (125f)"),
    }
}

impl Offset {
    pub fn to_seconds(self, fps: f64) -> f64 {
        self.seconds(fps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grammar() {
        assert_eq!(parse("1.2").unwrap(), TimeSpec::Moment(Moment::At(Offset::Seconds(1.2))));
        assert_eq!(parse("60f").unwrap(), TimeSpec::Moment(Moment::At(Offset::Frames(60))));
        assert_eq!(parse("settled").unwrap(), TimeSpec::Moment(Moment::Settled(Offset::Seconds(0.0))));
        assert_eq!(parse("out+0.2").unwrap(), TimeSpec::Moment(Moment::Marker("out".into(), Offset::Seconds(0.2))));
        assert_eq!(parse("stats-5f").unwrap(), TimeSpec::Moment(Moment::Marker("stats".into(), Offset::Frames(-5))));
        assert_eq!(
            parse("0..settled+1").unwrap(),
            TimeSpec::Range(Moment::At(Offset::Seconds(0.0)), Moment::Settled(Offset::Seconds(1.0)))
        );
        assert!(parse("-1").is_err());
        assert!(parse("1.2x").is_err());
        assert!(parse("out*2").is_err());
        assert!(cue_time("settled").is_err());
        assert_eq!(cue_time("125f").unwrap(), Offset::Frames(125));
    }
}
