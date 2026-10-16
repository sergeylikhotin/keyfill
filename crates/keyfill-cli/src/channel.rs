//! Канал: команды слоям, журнал пачек команд и снимок состояния.
//!
//! Команды исполняет сессия (`Session::command`): она держит JS, дерево и движение. Здесь —
//! то, что о канале можно сказать данными: какие команды бывают, как они пишутся в журнал и
//! что стоит на слоях.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Команда слою канала. Слой — число, порядок наложения: больше — выше.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "lowercase", deny_unknown_fields)]
pub enum Command {
    /// Создать экземпляр на слое, не показывая; его покажет `play` слоя.
    Load {
        layer: u32,
        component: String,
        #[serde(default, skip_serializing_if = "Map::is_empty")]
        props: Map<String, Value>,
    },
    /// С компонентом — `load` и сразу `play`. Без — показать загруженный: прежний экземпляр
    /// слоя уходит через `out`, новый входит.
    Play {
        layer: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        component: Option<String>,
        #[serde(default, skip_serializing_if = "Map::is_empty")]
        props: Map<String, Value>,
    },
    /// Снять остановку таймлайна: hold или цикл.
    Next { layer: u32 },
    /// Увести экземпляр слоя через `out`; после ухода он удаляется.
    Stop { layer: u32 },
    /// Новые значения пропсов; изменения идут через `transition`.
    Update { layer: u32, props: Map<String, Value> },
    /// Команда компонента с параметрами.
    Invoke {
        layer: u32,
        command: String,
        #[serde(default, skip_serializing_if = "Map::is_empty")]
        params: Map<String, Value>,
    },
    /// Убрать без анимации: слой или весь канал.
    Clear {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        layer: Option<u32>,
    },
}

impl Command {
    pub fn name(&self) -> &'static str {
        match self {
            Command::Load { .. } => "load",
            Command::Play { .. } => "play",
            Command::Next { .. } => "next",
            Command::Stop { .. } => "stop",
            Command::Update { .. } => "update",
            Command::Invoke { .. } => "invoke",
            Command::Clear { .. } => "clear",
        }
    }

    pub fn layer(&self) -> Option<u32> {
        match self {
            Command::Load { layer, .. }
            | Command::Play { layer, .. }
            | Command::Next { layer }
            | Command::Stop { layer }
            | Command::Update { layer, .. }
            | Command::Invoke { layer, .. } => Some(*layer),
            Command::Clear { layer } => *layer,
        }
    }

    /// Подпись для дампа и ошибок: `play 20 LowerThird`, `invoke 10 goal`.
    pub fn label(&self) -> String {
        let layer = self.layer().map(|l| format!(" {l}")).unwrap_or_default();
        let what = match self {
            Command::Load { component, .. } | Command::Play { component: Some(component), .. } => format!(" {component}"),
            Command::Invoke { command, .. } => format!(" {command}"),
            _ => String::new(),
        };
        format!("{}{layer}{what}", self.name())
    }
}

/// Команда в журнале: `id` — от клиента, для подтверждения; `error` — команда упала в JS
/// посреди исполнения. Такая команда остаётся в журнале: её частичный эффект уже в эфире, и
/// воспроизведение повторяет его.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(flatten)]
    pub cmd: Command,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl From<Command> for Entry {
    fn from(cmd: Command) -> Entry {
        Entry { id: None, cmd, error: None }
    }
}

/// Пачка команд: применяется атомарно, на одном кадре канала. Одиночная команда — пачка из
/// одной; take из preview (M4b) — пачка из всего подготовленного.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Batch {
    pub frame: u64,
    pub cmds: Vec<Entry>,
}

/// Первая строка журнала: что за канал и с какими часами и случайностью он шёл.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Header {
    /// Версия формата журнала.
    pub keyfill: u32,
    /// Файл канала (`defineChannel`), от каталога журнала.
    pub channel: String,
    #[serde(default = "one")]
    pub seed: u32,
    /// `Date.now()` сцены в миллисекундах.
    #[serde(default)]
    pub now: f64,
}

fn one() -> u32 {
    1
}

/// Журнал канала: заголовок и пачки по кадрам. Формат — JSON Lines (`*.journal.jsonl`).
#[derive(Debug, Clone, PartialEq)]
pub struct Journal {
    pub header: Header,
    pub batches: Vec<Batch>,
}

pub const JOURNAL_SUFFIX: &str = ".journal.jsonl";

pub fn is_journal(path: &Path) -> bool {
    path.to_string_lossy().ends_with(JOURNAL_SUFFIX)
}

/// Строка пачки: полная форма `{ frame, cmds: [...] }` или одна команда `{ frame, cmd, ... }`.
#[derive(Deserialize)]
#[serde(untagged)]
enum Line {
    Batch(Batch),
    One {
        frame: u64,
        #[serde(flatten)]
        entry: Entry,
    },
}

impl Journal {
    pub fn parse(text: &str) -> Result<Journal> {
        let mut lines = text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty());
        let (_, first) = lines.next().ok_or_else(|| crate::usage("журнал пуст: первая строка — заголовок {\"keyfill\":1,\"channel\":\"channel.tsx\"}"))?;
        let header: Header = serde_json::from_str(first).map_err(|e| crate::usage(format!("журнал, строка 1 (заголовок): {e}")))?;
        if header.keyfill != 1 {
            return Err(crate::usage(format!("журнал версии {}; этот keyfill читает версию 1", header.keyfill)));
        }
        let mut batches: Vec<Batch> = Vec::new();
        for (i, line) in lines {
            let batch = match serde_json::from_str::<Line>(line) {
                Ok(Line::Batch(b)) => b,
                Ok(Line::One { frame, entry }) => Batch { frame, cmds: vec![entry] },
                Err(_) => return Err(crate::usage(format!("журнал, строка {}: {}", i + 1, line_error(line)))),
            };
            if let Some(prev) = batches.last()
                && batch.frame < prev.frame
            {
                return Err(crate::usage(format!("журнал, строка {}: кадр {} раньше предыдущего {}", i + 1, batch.frame, prev.frame)));
            }
            batches.push(batch);
        }
        Ok(Journal { header, batches })
    }

    pub fn read(path: &Path) -> Result<Journal> {
        let text = std::fs::read_to_string(path).with_context(|| format!("журнал {}", crate::slash(path)))?;
        Journal::parse(&text).with_context(|| crate::slash(path))
    }

    /// Файл канала, на котором записан журнал.
    pub fn channel_path(&self, journal: &Path) -> PathBuf {
        journal.parent().unwrap_or(Path::new(".")).join(&self.header.channel)
    }

    pub fn to_jsonl(&self) -> String {
        let mut out = serde_json::to_string(&self.header).expect("заголовок сериализуется");
        out.push('\n');
        for b in &self.batches {
            out.push_str(&serde_json::to_string(b).expect("пачка сериализуется"));
            out.push('\n');
        }
        out
    }
}

/// Понятная причина, почему строка не разобралась: serde у `untagged` говорит только «не подошло».
fn line_error(line: &str) -> String {
    let v: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return format!("не JSON: {e}"),
    };
    if v.get("frame").and_then(Value::as_u64).is_none() {
        return "нет поля frame (номер кадра канала, целое ≥ 0)".into();
    }
    let one = |c: &Value| serde_json::from_value::<Entry>(c.clone()).err().map(|e| e.to_string());
    let err = match v.get("cmds") {
        Some(Value::Array(cmds)) => cmds.iter().find_map(one),
        Some(_) => Some("cmds — массив команд".into()),
        None => one(&v),
    };
    match err {
        Some(e) => format!("команда не разобралась: {e}; команды: load, play, next, stop, update, invoke, clear"),
        None => "строка не похожа ни на пачку { frame, cmds }, ни на команду { frame, cmd }".into(),
    }
}

/// Что делает экземпляр на слое.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstState {
    /// Загружен (`load`), ждёт play.
    Loaded,
    /// Показан: in, loop, hold.
    Playing,
    /// Ушёл в `out` после stop или замены; удалится, когда уход кончится.
    Leaving,
}

/// Экземпляр на слое.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Inst {
    /// `LowerThird#2`: так он называется в дампе, событиях и `inspect`.
    pub name: String,
    pub component: String,
    #[serde(skip)]
    pub root: u32,
    pub state: InstState,
    /// Текущие значения пропсов: после `update`, команд и `ctx.set`.
    pub props: Value,
}

/// Слой: показанный экземпляр, загруженный на смену и уходящие.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Layer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<Inst>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loaded: Option<Inst>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub leaving: Vec<Inst>,
}

impl Layer {
    pub fn is_empty(&self) -> bool {
        self.current.is_none() && self.loaded.is_none() && self.leaving.is_empty()
    }

    /// Все экземпляры слоя снизу вверх: уходящие, показанный, загруженный.
    pub fn all(&self) -> impl Iterator<Item = &Inst> {
        self.leaving.iter().chain(self.current.iter()).chain(self.loaded.iter())
    }
}

/// Снимок канала: что на каких слоях и с какими данными. Из него `/state` (M4b), preview
/// и восстановление после сбоя: `to_batch` — команды, которые ставят то же самое заново.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct State {
    pub layers: BTreeMap<u32, Layer>,
}

impl State {
    /// Команды, которые восстанавливают показанное и загруженное (уходящие уже не нужны).
    /// Восстановленные экземпляры проигрывают `in` заново; вход сразу в «эфир» — с M4b.
    pub fn to_batch(&self, frame: u64) -> Batch {
        let mut cmds = Vec::new();
        let props = |i: &Inst| i.props.as_object().cloned().unwrap_or_default();
        for (&layer, l) in &self.layers {
            if let Some(i) = &l.current {
                cmds.push(Command::Play { layer, component: Some(i.component.clone()), props: props(i) }.into());
            }
            if let Some(i) = &l.loaded {
                cmds.push(Command::Load { layer, component: i.component.clone(), props: props(i) }.into());
            }
        }
        Batch { frame, cmds }
    }
}

/// Итог команды для клиента: принята, принята с ошибкой JS или отклонена.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Ack {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub cmd: String,
    pub status: AckStatus,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AckStatus {
    /// Исполнена, записана в журнал.
    Ok,
    /// Не исполнялась: неверный слой, компонент, пропсы. В журнал не попала.
    Rejected,
    /// Упала в JS посреди исполнения: частичный эффект остался, команда в журнале с `error`.
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_roundtrip_and_short_form() {
        let text = r#"{"keyfill":1,"channel":"channel.tsx"}
{"frame":0,"cmd":"play","layer":20,"component":"LowerThird","props":{"name":"Иван"}}

{"frame":50,"cmds":[{"cmd":"update","layer":20,"props":{"name":"Пётр"}},{"id":"c3","cmd":"stop","layer":10}]}
"#;
        let j = Journal::parse(text).unwrap();
        assert_eq!(j.header.seed, 1);
        assert_eq!(j.batches.len(), 2);
        assert_eq!(j.batches[0].cmds[0].cmd.label(), "play 20 LowerThird");
        assert_eq!(j.batches[1].cmds[1].id.as_deref(), Some("c3"));
        assert_eq!(Journal::parse(&j.to_jsonl()).unwrap(), j);
    }

    #[test]
    fn journal_errors_name_the_line() {
        let head = r#"{"keyfill":1,"channel":"c.tsx"}"#;
        let err = |body: &str| Journal::parse(&format!("{head}\n{body}")).unwrap_err().to_string();
        assert!(err(r#"{"cmd":"stop","layer":1}"#).contains("строка 2: нет поля frame"));
        assert!(err(r#"{"frame":0,"cmd":"pause","layer":1}"#).contains("команды: load, play"));
        assert!(err("{\"frame\":5,\"cmd\":\"stop\",\"layer\":1}\n{\"frame\":2,\"cmd\":\"stop\",\"layer\":1}").contains("кадр 2 раньше предыдущего 5"));
        assert!(Journal::parse(r#"{"keyfill":2,"channel":"c.tsx"}"#).unwrap_err().to_string().contains("версии 2"));
    }

    #[test]
    fn state_restores_shown_and_loaded() {
        let inst = |name: &str, state| Inst { name: name.into(), component: "LowerThird".into(), root: 1, state, props: serde_json::json!({ "name": "Иван" }) };
        let mut s = State::default();
        s.layers.insert(20, Layer { current: Some(inst("LowerThird#1", InstState::Playing)), loaded: Some(inst("LowerThird#2", InstState::Loaded)), leaving: vec![inst("LowerThird#0", InstState::Leaving)] });
        let b = s.to_batch(7);
        let labels: Vec<String> = b.cmds.iter().map(|e| e.cmd.label()).collect();
        assert_eq!(labels, ["play 20 LowerThird", "load 20 LowerThird"]);
        assert_eq!(b.frame, 7);
    }
}
