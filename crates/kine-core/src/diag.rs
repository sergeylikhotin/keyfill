//! Диагностики в форме, удобной агенту: правило, адрес узла, место в исходнике, подсказка.

use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub level: Level,
    pub rule: String,
    pub node: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let level = match self.level {
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        };
        write!(f, "{level} {} {}: {}", self.rule, self.node, self.message)?;
        if let Some(src) = &self.source {
            write!(f, "\n    at {src}")?;
        }
        if let Some(hint) = &self.hint {
            write!(f, "\n    hint: {hint}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct Diagnostics(pub Vec<Diagnostic>);

impl Diagnostics {
    pub fn push(&mut self, level: Level, rule: &str, node: &str, source: Option<&str>, message: impl Into<String>, hint: Option<&str>) {
        self.0.push(Diagnostic {
            level,
            rule: rule.into(),
            node: node.into(),
            source: source.map(Into::into),
            message: message.into(),
            hint: hint.map(Into::into),
        });
    }
}
