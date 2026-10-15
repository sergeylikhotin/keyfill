//! Дерево сцены, которое собирается из команд JS.

use std::collections::BTreeMap;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use serde_json::Value;

use crate::diag::Diagnostic;

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Cmd {
    Create {
        id: u32,
        kind: String,
        #[serde(default)]
        src: Option<String>,
    },
    Set {
        id: u32,
        key: String,
        value: Value,
    },
    Insert {
        parent: u32,
        child: u32,
    },
    Text {
        id: u32,
        runs: Vec<Run>,
    },
    Theme {
        values: BTreeMap<String, Value>,
    },
    Stage {
        id: u32,
        #[serde(rename = "ref")]
        reference: [f32; 2],
        #[serde(default)]
        safe: Option<SafeConfig>,
        fps: f32,
        #[serde(default)]
        fonts: Vec<String>,
        #[serde(default, rename = "systemFonts")]
        system_fonts: Option<bool>,
    },
    /// Диагностика, которую обнаружил JS-рантайм (например, чтение сигнала вне трекинга).
    Diag(Diagnostic),
}

/// Кусок текста внутри `Text`: либо прямой текст, либо `Span` со своим `look`.
#[derive(Debug, Clone, Deserialize)]
pub struct Run {
    pub text: String,
    #[serde(default)]
    pub look: Option<Value>,
    #[serde(default)]
    pub src: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct SafeConfig {
    pub action: Option<f32>,
    pub title: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Stage,
    Box,
    Text,
    Image,
    Shape,
}

impl Kind {
    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "stage" => Kind::Stage,
            "box" => Kind::Box,
            "text" => Kind::Text,
            "image" => Kind::Image,
            "shape" => Kind::Shape,
            other => bail!("неизвестный тип узла {other:?}"),
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Stage => "stage",
            Kind::Box => "box",
            Kind::Text => "text",
            Kind::Image => "image",
            Kind::Shape => "shape",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Node {
    pub id: u32,
    pub kind: Kind,
    pub src: Option<String>,
    pub parent: Option<u32>,
    pub children: Vec<u32>,
    pub props: BTreeMap<String, Value>,
    /// Весь текст узла `Text` (склейка runs).
    pub text: String,
    pub runs: Vec<Run>,
}

impl Node {
    pub fn hidden(&self) -> bool {
        self.props.get("hidden").and_then(Value::as_bool).unwrap_or(false)
    }

    /// Имя экземпляра компонента, если узел — корень компонента (`LowerThird#1`).
    pub fn instance(&self) -> Option<&str> {
        self.props.get("instance")?.get("name")?.as_str()
    }

    /// Часть компонента: (`LowerThird#1`, `panel`).
    pub fn part(&self) -> Option<(&str, &str)> {
        let p = self.props.get("part")?;
        Some((p.get("inst")?.as_str()?, p.get("part")?.as_str()?))
    }
}

#[derive(Debug, Clone)]
pub struct Stage {
    pub id: u32,
    pub size: [f32; 2],
    pub action: f32,
    pub title: f32,
    pub fps: f32,
    /// Пути к файлам шрифтов, относительно каталога файла сцены.
    pub fonts: Vec<String>,
    pub system_fonts: bool,
}

#[derive(Debug, Default)]
pub struct Scene {
    pub nodes: BTreeMap<u32, Node>,
    pub stage: Option<Stage>,
    pub theme: BTreeMap<String, Value>,
    pub js_diagnostics: Vec<Diagnostic>,
}

impl Scene {
    pub fn apply_batch(&mut self, json: &str) -> Result<()> {
        let cmds: Vec<Cmd> = serde_json::from_str(json).context("пакет команд из JS не разобрался")?;
        for cmd in cmds {
            self.apply(cmd)?;
        }
        Ok(())
    }

    pub fn apply(&mut self, cmd: Cmd) -> Result<()> {
        match cmd {
            Cmd::Create { id, kind, src } => {
                let node = Node {
                    id,
                    kind: Kind::parse(&kind)?,
                    src,
                    parent: None,
                    children: Vec::new(),
                    props: BTreeMap::new(),
                    text: String::new(),
                    runs: Vec::new(),
                };
                self.nodes.insert(id, node);
            }
            Cmd::Set { id, key, value } => {
                self.node_mut(id)?.props.insert(key, value);
            }
            Cmd::Insert { parent, child } => {
                if let Some(old) = self.node_mut(child)?.parent.take() {
                    self.node_mut(old)?.children.retain(|c| *c != child);
                }
                self.node_mut(parent)?.children.push(child);
                self.node_mut(child)?.parent = Some(parent);
            }
            Cmd::Text { id, runs } => {
                let node = self.node_mut(id)?;
                node.text = runs.iter().map(|r| r.text.as_str()).collect();
                node.runs = runs;
            }
            Cmd::Diag(d) => self.js_diagnostics.push(d),
            Cmd::Theme { values } => {
                self.theme.extend(values);
            }
            Cmd::Stage { id, reference, safe, fps, fonts, system_fonts } => {
                let safe = safe.unwrap_or_default();
                self.stage = Some(Stage {
                    id,
                    size: reference,
                    action: safe.action.unwrap_or(0.93),
                    title: safe.title.unwrap_or(0.90),
                    fps,
                    fonts,
                    system_fonts: system_fonts.unwrap_or(true),
                });
            }
        }
        Ok(())
    }

    fn node_mut(&mut self, id: u32) -> Result<&mut Node> {
        self.nodes.get_mut(&id).with_context(|| format!("команда ссылается на несуществующий узел {id}"))
    }

    pub fn node(&self, id: u32) -> &Node {
        &self.nodes[&id]
    }

    /// Адрес узла для дампа, ошибок и inspect: `LowerThird#1/title`, `LowerThird#1/panel/box[1]`.
    pub fn path(&self, id: u32) -> String {
        let node = self.node(id);
        if node.kind == Kind::Stage {
            return "stage".into();
        }
        if let Some((inst, part)) = node.part() {
            return format!("{inst}/{part}");
        }
        if let Some(inst) = node.instance() {
            return inst.to_string();
        }
        let Some(parent) = node.parent else {
            return format!("{}?{id}", node.kind.name());
        };
        let siblings = &self.node(parent).children;
        let index = siblings
            .iter()
            .filter(|c| self.node(**c).kind == node.kind)
            .position(|c| *c == id)
            .unwrap_or(0);
        format!("{}/{}[{index}]", self.path(parent), node.kind.name())
    }

    /// Ищет узел по адресу; корень компонента находится и по имени экземпляра.
    pub fn find(&self, path: &str) -> Option<u32> {
        self.nodes
            .values()
            .find(|n| self.path(n.id) == path || n.instance() == Some(path))
            .map(|n| n.id)
    }
}
