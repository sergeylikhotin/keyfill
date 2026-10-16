//! Хореография переходов канала: правила (`together`, `sequence`, шаги с `after`/`with`/`at`)
//! и расписание перехода — когда начинается каждый шаг. Расписание считается один раз, в момент
//! команды `state`, по длительностям шагов; дамп показывает его диаграммой.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::look::num;

/// Что происходит со слоем в переходе.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StepKind {
    In,
    Out,
    Update,
}

impl StepKind {
    pub fn name(self) -> &'static str {
        match self {
            StepKind::In => "in",
            StepKind::Out => "out",
            StepKind::Update => "update",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AnchorRel {
    /// От конца шага.
    After,
    /// От начала шага.
    With,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Anchor {
    pub rel: AnchorRel,
    pub layer: u32,
    pub kind: StepKind,
}

/// Шаг правила: какой шаг какого слоя и когда он начинается.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct StepDef {
    pub layer: u32,
    pub kind: StepKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Anchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<f64>,
    #[serde(default)]
    pub offset: f64,
}

/// Правило перехода.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Rule {
    /// Всё в момент команды.
    Together,
    /// `in` через `gap` после конца всех `out` (отрицательный — раньше конца: `overlap`).
    Sequence { gap: f64 },
    /// Шаги со своими моментами; не названные — в начале перехода.
    Steps { steps: Vec<StepDef> },
}

impl Rule {
    pub fn label(&self) -> String {
        match self {
            Rule::Together => "together".into(),
            Rule::Sequence { gap } if *gap < 0.0 => format!("overlap {}", num(-*gap as f32)),
            Rule::Sequence { gap } => format!("sequence +{}", num(*gap as f32)),
            Rule::Steps { .. } => "шаги".into(),
        }
    }
}

/// Правило для пары состояний: `from -> to`, `*` — любое.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct TransitionDef {
    pub from: String,
    pub to: String,
    pub rule: Rule,
}

/// Правило перехода `from → to`: точная пара, затем `from -> *`, `* -> to`, `* -> *`, `default`,
/// иначе `together`. Подпись — откуда правило.
pub fn pick(transitions: &[TransitionDef], default: Option<&Rule>, from: Option<&str>, to: &str) -> (Rule, String) {
    let from = from.unwrap_or("");
    let find = |f: &str, t: &str| transitions.iter().find(|d| d.from == f && d.to == t);
    let found = find(from, to).or_else(|| find(from, "*")).or_else(|| find("*", to)).or_else(|| find("*", "*"));
    match (found, default) {
        (Some(d), _) => (d.rule.clone(), format!("{} -> {}", d.from, d.to)),
        (None, Some(r)) => (r.clone(), "default".into()),
        (None, None) => (Rule::Together, "по умолчанию".into()),
    }
}

/// Шаг этого перехода: слой, что с ним происходит, длительность.
#[derive(Debug, Clone, PartialEq)]
pub struct Act {
    pub layer: u32,
    pub kind: StepKind,
    pub dur: f64,
}

/// Начало шага от начала перехода и почему именно тогда (для диаграммы).
#[derive(Debug, Clone, PartialEq)]
pub struct Timed {
    pub start: f64,
    pub why: Option<String>,
}

/// Расписание перехода: момент начала каждого шага `acts` от начала перехода (≥ 0).
pub fn resolve(rule: &Rule, acts: &[Act], names: &BTreeMap<u32, String>) -> Vec<Timed> {
    let at0 = || Timed { start: 0.0, why: None };
    let label = |layer: u32, kind: StepKind| format!("{}:{}", names.get(&layer).cloned().unwrap_or_else(|| layer.to_string()), kind.name());
    match rule {
        Rule::Together => acts.iter().map(|_| at0()).collect(),
        Rule::Sequence { gap } => {
            let outs = acts.iter().filter(|a| a.kind == StepKind::Out).map(|a| a.dur).reduce(f64::max);
            acts.iter()
                .map(|a| match (a.kind, outs) {
                    (StepKind::In, Some(end)) => Timed { start: (end + gap).max(0.0), why: Some(format!("после всех out {}", signed(*gap))) },
                    _ => at0(),
                })
                .collect()
        }
        Rule::Steps { steps } => {
            let step = |a: &Act| steps.iter().find(|s| s.layer == a.layer && s.kind == a.kind);
            let index = |layer: u32, kind: StepKind| acts.iter().position(|a| a.layer == layer && a.kind == kind);
            let mut out: Vec<Option<Timed>> = vec![None; acts.len()];
            // Шаг ждёт свой якорь; без якоря, с якорем, которого в переходе нет, или в цикле — от начала.
            for _ in 0..=acts.len() {
                let mut progress = false;
                for i in 0..acts.len() {
                    if out[i].is_some() {
                        continue;
                    }
                    let timed = match step(&acts[i]) {
                        None => Some(at0()),
                        Some(s) => match (&s.anchor, s.at) {
                            (None, Some(at)) => Some(Timed { start: at + s.offset, why: Some(format!("at {}", num((at + s.offset) as f32))) }),
                            (None, None) => Some(Timed { start: s.offset, why: (s.offset != 0.0).then(|| format!("offset {}", signed(s.offset))) }),
                            (Some(a), _) => {
                                let what = format!("{} {}", if a.rel == AnchorRel::After { "after" } else { "with" }, label(a.layer, a.kind));
                                match index(a.layer, a.kind) {
                                    None => Some(Timed { start: s.offset, why: Some(format!("{what}: такого шага нет, от начала")) }),
                                    Some(j) => out[j].as_ref().map(|tj| {
                                        let base = if a.rel == AnchorRel::After { tj.start + acts[j].dur } else { tj.start };
                                        let off = if s.offset != 0.0 { format!(" {}", signed(s.offset)) } else { String::new() };
                                        Timed { start: base + s.offset, why: Some(format!("{what}{off}")) }
                                    }),
                                }
                            }
                        },
                    };
                    if let Some(mut tm) = timed {
                        tm.start = tm.start.max(0.0);
                        out[i] = Some(tm);
                        progress = true;
                    }
                }
                if !progress {
                    break;
                }
            }
            out.into_iter().map(|t| t.unwrap_or(Timed { start: 0.0, why: Some("цикл, от начала".into()) })).collect()
        }
    }
}

fn signed(v: f64) -> String {
    if v < 0.0 { format!("−{}", num(-v as f32)) } else { format!("+{}", num(v as f32)) }
}

/// Шаг расписания для дампа.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlanStep {
    pub layer: u32,
    /// Имя слоя из `layers`.
    pub name: String,
    pub kind: StepKind,
    pub inst: String,
    /// От начала перехода, секунды.
    pub start: f64,
    pub end: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// Расписание перехода, как его свело ядро в момент команды.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TransitionPlan {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    pub to: String,
    /// Момент команды.
    pub t: f64,
    /// Правило и откуда оно: `sequence +0.1 (default)`.
    pub rule: String,
    pub steps: Vec<PlanStep>,
}

impl TransitionPlan {
    /// Когда кончится последний шаг (абсолютное время).
    pub fn end(&self) -> f64 {
        self.t + self.steps.iter().map(|s| s.end).fold(0.0, f64::max)
    }

    /// Диаграмма: шаг на строку, времена от начала перехода.
    pub fn lines(&self) -> Vec<String> {
        let mut out = vec![format!(
            "переход {} → {}  @{}s  ({})",
            self.from.as_deref().unwrap_or("—"),
            self.to,
            num(self.t as f32),
            self.rule
        )];
        let width = self.steps.iter().map(|s| s.name.chars().count()).max().unwrap_or(0);
        for s in &self.steps {
            let span = if s.end > s.start { format!("{}–{}", num(s.start as f32), num(s.end as f32)) } else { num(s.start as f32) };
            let mut line = format!("  {:<width$}  {:<6}  {:<10}  {}", s.name, s.kind.name(), span, s.inst);
            if let Some(w) = &s.why {
                let _ = write!(line, "  ({w})");
            }
            out.push(line);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> BTreeMap<u32, String> {
        [(10, "score".to_string()), (20, "lower".to_string()), (30, "replay".to_string())].into()
    }

    fn act(layer: u32, kind: StepKind, dur: f64) -> Act {
        Act { layer, kind, dur }
    }

    #[test]
    fn sequence_and_overlap_follow_outs() {
        let acts = [act(20, StepKind::Out, 0.3), act(30, StepKind::In, 0.5), act(10, StepKind::Update, 0.0)];
        let starts = |r: Rule| resolve(&r, &acts, &names()).iter().map(|t| t.start).collect::<Vec<_>>();
        assert_eq!(starts(Rule::Together), [0.0, 0.0, 0.0]);
        assert_eq!(starts(Rule::Sequence { gap: 0.1 }), [0.0, 0.4, 0.0]);
        let s = starts(Rule::Sequence { gap: -0.15 });
        assert!((s[1] - 0.15).abs() < 1e-12, "{s:?}");
        assert_eq!(starts(Rule::Sequence { gap: -1.0 })[1], 0.0, "не раньше начала перехода");
    }

    #[test]
    fn steps_chain_after_and_with() {
        // replay входит за 0.1 до конца ухода lower, score обновляется вместе с входом replay.
        let acts = [act(20, StepKind::Out, 0.25), act(30, StepKind::In, 0.45), act(10, StepKind::Update, 0.0)];
        let step = |layer, kind, anchor: Option<Anchor>, offset| StepDef { layer, kind, anchor, at: None, offset };
        let rule = Rule::Steps {
            steps: vec![
                step(10, StepKind::Update, Some(Anchor { rel: AnchorRel::With, layer: 30, kind: StepKind::In }), 0.05),
                step(30, StepKind::In, Some(Anchor { rel: AnchorRel::After, layer: 20, kind: StepKind::Out }), -0.1),
            ],
        };
        let t = resolve(&rule, &acts, &names());
        assert_eq!(t[0].start, 0.0);
        assert!((t[1].start - 0.15).abs() < 1e-12 && t[1].why.as_deref() == Some("after lower:out −0.1"), "{t:?}");
        assert!((t[2].start - 0.2).abs() < 1e-12 && t[2].why.as_deref() == Some("with replay:in +0.05"), "{t:?}");
        // Якоря нет в этом переходе (прерывание пересобрало разницу) — от начала, с пометкой.
        let t = resolve(&rule, &acts[1..], &names());
        assert_eq!(t[0].start, 0.0);
        assert!(t[0].why.as_deref().unwrap().contains("такого шага нет"), "{t:?}");
    }

    #[test]
    fn rule_lookup_prefers_exact_pairs() {
        let d = |from: &str, to: &str, gap| TransitionDef { from: from.into(), to: to.into(), rule: Rule::Sequence { gap } };
        let all = [d("*", "*", 1.0), d("*", "replay", 2.0), d("live", "*", 3.0), d("live", "replay", 4.0)];
        let gap = |from: Option<&str>, to: &str, list: &[TransitionDef]| match pick(list, None, from, to).0 {
            Rule::Sequence { gap } => gap,
            _ => 0.0,
        };
        assert_eq!(gap(Some("live"), "replay", &all), 4.0);
        assert_eq!(gap(Some("live"), "break", &all), 3.0);
        assert_eq!(gap(Some("pregame"), "replay", &all), 2.0);
        assert_eq!(gap(None, "break", &all), 1.0);
        assert_eq!(pick(&[], None, None, "live").0, Rule::Together);
    }
}
