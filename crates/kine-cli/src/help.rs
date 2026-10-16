//! Встроенная справка: `kine explain <правило>` и `kine docs <тема>`.
//!
//! Тексты лежат в `help/` и встраиваются в бинарь, поэтому всегда совпадают с его версией.
//! `help/rules.md` — по разделу `## <правило>` на каждое правило диагностики; тест
//! `every_rule_has_explain` сверяет разделы с правилами в исходниках kine.

use serde::Serialize;

const RULES: &str = include_str!("../help/rules.md");

/// Темы `kine docs` в порядке списка.
const TOPICS: &[(&str, &str)] = &[
    ("start", include_str!("../help/docs/start.md")),
    ("cli", include_str!("../help/docs/cli.md")),
    ("time", include_str!("../help/docs/time.md")),
    ("scene", include_str!("../help/docs/scene.md")),
    ("cues", include_str!("../help/docs/cues.md")),
    ("look", include_str!("../help/docs/look.md")),
    ("layout", include_str!("../help/docs/layout.md")),
    ("must", include_str!("../help/docs/must.md")),
    ("motion", include_str!("../help/docs/motion.md")),
    ("timeline", include_str!("../help/docs/timeline.md")),
    ("test", include_str!("../help/docs/test.md")),
    ("config", include_str!("../help/docs/config.md")),
    ("perf", include_str!("../help/docs/perf.md")),
];

/// Объяснение правила диагностики.
#[derive(Debug, Clone, Serialize)]
pub struct RuleDoc {
    pub rule: String,
    /// Обычный уровень: `error`, `warn`, `info`.
    pub level: String,
    pub title: String,
    pub body: String,
}

/// Тема справки.
#[derive(Debug, Clone, Serialize)]
pub struct Topic {
    pub topic: String,
    pub title: String,
    pub body: String,
}

/// Все правила из `help/rules.md` в порядке файла.
pub fn rules() -> Vec<RuleDoc> {
    let mut out = Vec::new();
    for section in RULES.split("\n## ").skip(1) {
        let (rule, rest) = section.split_once('\n').unwrap_or((section, ""));
        let (head, body) = rest.split_once('\n').unwrap_or((rest, ""));
        let (level, title) = head.split_once(" · ").unwrap_or(("", head));
        out.push(RuleDoc { rule: rule.trim().into(), level: level.trim().into(), title: title.trim().into(), body: body.trim().into() });
    }
    out
}

pub fn rule(id: &str) -> Option<RuleDoc> {
    rules().into_iter().find(|r| r.rule == id)
}

/// Правила, похожие на неизвестное имя: то же пространство (`must.`) или общая подстрока.
pub fn similar(id: &str) -> Vec<String> {
    let ns = id.split('.').next().unwrap_or(id);
    rules()
        .into_iter()
        .map(|r| r.rule)
        .filter(|r| r.starts_with(&format!("{ns}.")) || r.contains(id) || id.contains(r.as_str()))
        .collect()
}

pub fn topics() -> Vec<Topic> {
    TOPICS
        .iter()
        .map(|(name, text)| {
            let (head, body) = text.split_once('\n').unwrap_or((text, ""));
            let title = head.trim_start_matches('#').trim();
            let title = title.split_once(" — ").map_or(title, |(_, t)| t);
            Topic { topic: (*name).into(), title: title.into(), body: body.trim().into() }
        })
        .collect()
}

pub fn topic(name: &str) -> Option<Topic> {
    topics().into_iter().find(|t| t.topic == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_parse_with_level_and_title() {
        let fit = rule("must.fit").unwrap();
        assert_eq!(fit.level, "error");
        assert!(fit.title.contains("не помещается"));
        assert!(fit.body.contains("shrinkTo"));
        for r in rules() {
            assert!(["error", "warn", "info"].contains(&r.level.as_str()), "{}: уровень {:?}", r.rule, r.level);
            assert!(!r.title.is_empty() && !r.body.is_empty(), "{}: пустой раздел", r.rule);
        }
    }

    #[test]
    fn topics_have_titles() {
        for t in topics() {
            assert!(!t.title.is_empty() && !t.body.is_empty(), "{}", t.topic);
        }
        assert_eq!(topic("time").unwrap().title, "грамматика времени");
    }
}
