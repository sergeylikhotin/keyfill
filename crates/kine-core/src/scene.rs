//! Дерево сцены, которое собирается из команд JS.

use std::collections::BTreeMap;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::diag::{Diagnostic, Diagnostics, Level};
use crate::motion::Compiler;
use crate::timeline::{self, TimelineDef};

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
    /// Сценарий команд из `*.cues.ts`.
    Cues { items: Vec<Cue> },
    /// Таймлайны из `defineScene({ timelines })`.
    Timelines { items: Vec<Value> },
    /// play / next / stop / emit от команды компонента.
    Motion(MotionCmd),
    /// Схема пропсов сцены и ошибки входных данных (`--props`, `--set`).
    Schema {
        props: Vec<PropInfo>,
        #[serde(default)]
        errors: Vec<String>,
    },
    /// Описание компонента: пропсы, части, команды.
    Component(ComponentInfo),
}

/// Пропс сцены, компонента или параметр команды.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct PropInfo {
    pub name: String,
    /// `string`, `number`, `boolean` или `json` (тип из `prop<T>()`).
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct CommandInfo {
    pub name: String,
    pub params: Vec<PropInfo>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ComponentInfo {
    pub name: String,
    pub props: Vec<PropInfo>,
    #[serde(default)]
    pub parts: Vec<String>,
    #[serde(default)]
    pub commands: Vec<CommandInfo>,
}

/// Команда движения, которую отправила команда компонента (`ctx.play/next/stop/emit`); ядро
/// исполняет её в момент команды сценария.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MotionCmd {
    pub inst: String,
    #[serde(flatten)]
    pub action: MotionAction,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "cmd", rename_all = "lowercase")]
pub enum MotionAction {
    /// Без маркера — play экземпляра; с маркером — сегмент таймлайна от маркера до следующего.
    Play {
        #[serde(default)]
        marker: Option<String>,
    },
    Next,
    Stop,
    /// Своё событие для пульта и журнала.
    Emit { name: String },
}

/// Команда сценария (`*.cues.ts`, `--cue`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Cue {
    pub t: f64,
    /// Компонент: команда только для его экземпляров; у `update` без компонента — пропсы сцены.
    #[serde(default)]
    pub target: Option<String>,
    #[serde(flatten)]
    pub cmd: CueCmd,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "cmd", rename_all = "lowercase")]
pub enum CueCmd {
    Play,
    Stop,
    Next,
    /// Новые значения пропсов: экземпляров компонента или самой сцены.
    Update {
        #[serde(default)]
        props: Option<Value>,
    },
    /// Команда компонента с параметрами.
    Invoke {
        command: String,
        #[serde(default)]
        props: Option<Value>,
    },
}

impl CueCmd {
    pub fn name(&self) -> &'static str {
        match self {
            CueCmd::Play => "play",
            CueCmd::Stop => "stop",
            CueCmd::Next => "next",
            CueCmd::Update { .. } => "update",
            CueCmd::Invoke { .. } => "invoke",
        }
    }
}

impl Cue {
    pub fn label(&self) -> String {
        let t = crate::look::num(self.t as f32);
        let cmd = self.cmd.name();
        match (&self.target, &self.cmd) {
            (Some(target), CueCmd::Invoke { command, .. }) => format!("{cmd}@{t} {target}.{command}"),
            (Some(target), _) => format!("{cmd}@{t} {target}"),
            _ => format!("{cmd}@{t}"),
        }
    }
}

/// Кусок текста внутри `Text`: либо прямой текст, либо `Span` со своим `look`.
#[derive(Debug, Clone, Deserialize)]
pub struct Run {
    pub text: String,
    #[serde(default)]
    pub look: Option<Value>,
    /// `count()` или `clock()`: текст куска считает ядро на каждом кадре.
    #[serde(default)]
    pub dynamic: Option<Value>,
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
    /// Абсолютные пути к файлам шрифтов (бандлер разрешает их из import).
    pub fonts: Vec<String>,
    pub system_fonts: bool,
}

#[derive(Debug, Default)]
pub struct Scene {
    pub nodes: BTreeMap<u32, Node>,
    pub stage: Option<Stage>,
    pub theme: BTreeMap<String, Value>,
    pub js_diagnostics: Vec<Diagnostic>,
    pub cues: Vec<Cue>,
    /// Таймлайны из `defineScene({ timelines })`, разобранные один раз при монтировании.
    pub timelines: Vec<TimelineDef>,
    /// Ошибки и замечания разбора таймлайнов.
    pub timeline_diagnostics: Diagnostics,
    /// Команды движения от JS, ещё не исполненные: их забирает сессия после каждого пакета.
    pub motion_cmds: Vec<MotionCmd>,
    /// Пропсы сцены (`defineScene({ props, defaults })`).
    pub props: Vec<PropInfo>,
    /// Ошибки входных пропсов: неизвестное имя, неверный тип, не задан обязательный.
    pub props_errors: Vec<String>,
    pub components: Vec<ComponentInfo>,
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
            Cmd::Cues { items } => self.cues.extend(items),
            Cmd::Timelines { items } => self.add_timelines(&items),
            Cmd::Motion(m) => self.motion_cmds.push(m),
            Cmd::Schema { props, errors } => {
                self.props = props;
                self.props_errors = errors;
            }
            Cmd::Component(c) => self.components.push(c),
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

    /// Разбирает таймлайны: значения ключей компилируются по теме, ошибки становятся диагностиками.
    fn add_timelines(&mut self, items: &[Value]) {
        for v in items {
            let mut c = Compiler { theme: &self.theme, notes: Vec::new() };
            let name = v.get("timeline").and_then(Value::as_str).unwrap_or("?").to_string();
            let diags = &mut self.timeline_diagnostics;
            match timeline::parse(v, &mut c) {
                Ok(d) if self.timelines.iter().any(|x| x.component == d.component) => {
                    diags.push(Level::Error, "timeline.duplicate", &name, None, "у компонента два таймлайна; используется первый", None);
                }
                Ok(d) => self.timelines.push(d),
                Err(errors) => {
                    for e in errors {
                        diags.push(Level::Error, "timeline.invalid", &name, None, e, None);
                    }
                }
            }
            for (level, rule, msg) in c.notes {
                diags.push(level, rule, &name, None, msg, None);
            }
        }
    }

    /// Таймлайн компонента.
    pub fn timeline(&self, component: &str) -> Option<&TimelineDef> {
        self.timelines.iter().find(|t| t.component == component)
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
