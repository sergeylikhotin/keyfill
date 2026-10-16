//! QuickJS-хост: исполняет бандл сцены и собирает пакеты команд, которые JS шлёт в ядро.
//!
//! Хост живёт всю сессию: после монтирования ядро вызывает JS на командах сценария
//! (`update`), а не на каждом кадре. JS не видит реальное время и настоящую случайность:
//! `Date.now`, `performance.now` и `Math.random` подменены до запуска бандла, таймеров нет —
//! время ведёт ядро.

use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Result;
use rquickjs::{CaughtError, CatchResultExt, Context, Function, Runtime};

#[derive(Debug, Clone)]
pub struct RunOptions {
    /// Сид для `Math.random`.
    pub seed: u32,
    /// Значение `Date.now()` и `performance.now()` в миллисекундах.
    pub now_ms: f64,
    /// Пропсы сцены из CLI, JSON `{ values, raw }`: `values` — из `--props`, `raw` — строки
    /// из `--set`, их приводит к типам схемы сама сцена.
    pub props: Option<String>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self { seed: 1, now_ms: 0.0, props: None }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogLine {
    pub level: String,
    pub message: String,
}

#[derive(Debug, Default)]
pub struct RunOutput {
    /// Пакеты команд в порядке отправки, каждый — JSON-массив.
    pub batches: Vec<String>,
    pub logs: Vec<LogLine>,
}

const PRELUDE: &str = r#"
(() => {
  const fmt = (args) => args.map((a) => (typeof a === 'string' ? a : JSON.stringify(a))).join(' ');
  globalThis.console = {
    log: (...a) => __kine_log('log', fmt(a)),
    info: (...a) => __kine_log('info', fmt(a)),
    warn: (...a) => __kine_log('warn', fmt(a)),
    error: (...a) => __kine_log('error', fmt(a)),
  };
  let s = __KINE_SEED__ >>> 0;
  Math.random = () => {
    s = (Math.imul(s, 1664525) + 1013904223) >>> 0;
    return s / 4294967296;
  };
  Date.now = () => __KINE_NOW__;
  globalThis.performance = { now: () => __KINE_NOW__ };
  globalThis.__kine_props = __KINE_PROPS__;
  const noTimers = () => {
    throw new Error('kine: таймеров нет, время ведёт ядро: движение — presence/transition/timeline, смена данных — команды сценария (cues)');
  };
  globalThis.setTimeout = noTimers;
  globalThis.setInterval = noTimers;
  globalThis.requestAnimationFrame = noTimers;
})();
"#;

/// Живой JS-рантайм сцены.
pub struct JsHost {
    // Контекст держит рантайм; рантайм должен пережить контекст.
    ctx: Context,
    _rt: Runtime,
    out: Rc<RefCell<RunOutput>>,
}

impl JsHost {
    /// Запускает бандл; возвращает хост и то, что сцена отправила при монтировании.
    pub fn start(source: &str, opts: &RunOptions) -> Result<(JsHost, RunOutput)> {
        let rt = Runtime::new()?;
        let ctx = Context::full(&rt)?;
        let out = Rc::new(RefCell::new(RunOutput::default()));
        init(&ctx, &out, source, opts)?;
        let first = std::mem::take(&mut *out.borrow_mut());
        Ok((JsHost { ctx, _rt: rt, out }, first))
    }

    /// Вызывает глобальную функцию сцены с JSON-аргументом и возвращает отправленные пакеты.
    pub fn call(&mut self, name: &str, arg_json: &str) -> Result<RunOutput> {
        let result: Result<()> = self.ctx.with(|ctx| {
            let f: Function = ctx.globals().get(name).catch(&ctx).map_err(js_error)?;
            f.call::<_, ()>((arg_json.to_string(),)).catch(&ctx).map_err(js_error)?;
            Ok(())
        });
        result?;
        Ok(std::mem::take(&mut *self.out.borrow_mut()))
    }
}

fn init(ctx: &Context, out: &Rc<RefCell<RunOutput>>, source: &str, opts: &RunOptions) -> Result<()> {
    ctx.with(|ctx| {
        let globals = ctx.globals();

        let sink = out.clone();
        globals.set(
            "__kine_emit",
            Function::new(ctx.clone(), move |json: String| sink.borrow_mut().batches.push(json))?,
        )?;
        let sink = out.clone();
        globals.set(
            "__kine_log",
            Function::new(ctx.clone(), move |level: String, message: String| {
                sink.borrow_mut().logs.push(LogLine { level, message })
            })?,
        )?;

        let prelude = PRELUDE
            .replace("__KINE_SEED__", &opts.seed.to_string())
            .replace("__KINE_NOW__", &format!("{:?}", opts.now_ms))
            .replace("__KINE_PROPS__", opts.props.as_deref().unwrap_or("null"));
        ctx.eval::<(), _>(prelude).catch(&ctx).map_err(js_error)?;
        ctx.eval::<(), _>(source).catch(&ctx).map_err(js_error)?;
        Ok(())
    })
}

/// Исключение в JS сцены. Отдельный тип, чтобы CLI отличал его от сбоя самого kine;
/// стек разобран на кадры здесь, чтобы sourcemap и диагностика работали с полями, а не с текстом.
#[derive(Debug, Clone, PartialEq)]
pub struct JsException {
    pub message: String,
    /// Ошибка API kine (`KineError`): место в TSX известно, стек бандла не нужен.
    pub kine: bool,
    /// Место в исходнике: у `KineError` — из dev-JSX, иначе первый кадр стека после sourcemap.
    pub source: Option<String>,
    pub stack: Vec<Frame>,
    /// Сколько кадров скрыто как внутренности `@kine/core`.
    pub hidden: usize,
}

/// Кадр стека: функция и место, строки и столбцы с единицы.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub function: String,
    pub file: String,
    pub line: u32,
    pub column: u32,
}

impl std::fmt::Display for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "at {} ({}:{}:{})", self.function, self.file, self.line, self.column)
    }
}

impl Frame {
    /// Строка стека QuickJS: `    at f (eval_script:12:5)` или `    at eval_script:12:5`.
    fn parse(line: &str) -> Option<Frame> {
        let rest = line.trim().strip_prefix("at ")?;
        let (function, loc) = match rest.rfind(" (") {
            Some(i) if rest.ends_with(')') => (&rest[..i], &rest[i + 2..rest.len() - 1]),
            _ => ("<anonymous>", rest),
        };
        let mut parts = loc.rsplitn(3, ':');
        let column = parts.next()?.parse().ok()?;
        let line = parts.next()?.parse().ok()?;
        Some(Frame { function: function.to_string(), file: parts.next()?.to_string(), line, column })
    }
}

impl std::fmt::Display for JsException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.kine {
            f.write_str(&self.message)?;
            if let Some(src) = &self.source {
                write!(f, "\n  at {src}")?;
            }
            return Ok(());
        }
        write!(f, "JS: {}", self.message)?;
        for frame in &self.stack {
            write!(f, "\n    {frame}")?;
        }
        if self.hidden > 0 {
            write!(f, "\n    … ещё {} в @kine/core", self.hidden)?;
        }
        Ok(())
    }
}

impl std::error::Error for JsException {}

fn js_error(e: CaughtError<'_>) -> anyhow::Error {
    let ex = match e {
        CaughtError::Exception(ex) => {
            let name: Option<String> = ex.get("name").ok();
            let kine = name.as_deref() == Some("KineError");
            JsException {
                message: ex.message().unwrap_or_default(),
                kine,
                source: if kine { ex.get::<_, Option<String>>("src").ok().flatten() } else { None },
                stack: if kine { Vec::new() } else { ex.stack().unwrap_or_default().lines().filter_map(Frame::parse).collect() },
                hidden: 0,
            }
        }
        other => JsException { message: other.to_string(), kine: false, source: None, stack: Vec::new(), hidden: 0 },
    };
    anyhow::Error::new(ex)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_bundle(source: &str, opts: &RunOptions) -> Result<RunOutput> {
        JsHost::start(source, opts).map(|(_, out)| out)
    }

    #[test]
    fn collects_batches_and_logs() {
        let out = run_bundle("console.log('hi', 1); __kine_emit('[1]'); __kine_emit('[2]');", &RunOptions::default()).unwrap();
        assert_eq!(out.batches, vec!["[1]", "[2]"]);
        assert_eq!(out.logs, vec![LogLine { level: "log".into(), message: "hi 1".into() }]);
    }

    #[test]
    fn time_and_random_are_deterministic() {
        let src = "__kine_emit(JSON.stringify([Date.now(), performance.now(), Math.random(), Math.random()]));";
        let opts = RunOptions { seed: 42, now_ms: 1500.0, ..Default::default() };
        let a = run_bundle(src, &opts).unwrap();
        let b = run_bundle(src, &opts).unwrap();
        assert_eq!(a.batches, b.batches);
        assert!(a.batches[0].starts_with("[1500,1500,"));
    }

    #[test]
    fn timers_are_rejected_with_hint() {
        let err = run_bundle("setTimeout(() => {}, 10)", &RunOptions::default()).unwrap_err();
        assert!(err.to_string().contains("время ведёт ядро"), "{err}");
    }

    #[test]
    fn host_keeps_state_between_calls() {
        let src = "let n = 0; globalThis.bump = (json) => { n += JSON.parse(json).by; __kine_emit(String(n)); };";
        let (mut host, first) = JsHost::start(src, &RunOptions::default()).unwrap();
        assert!(first.batches.is_empty());
        assert_eq!(host.call("bump", r#"{"by":2}"#).unwrap().batches, vec!["2"]);
        assert_eq!(host.call("bump", r#"{"by":3}"#).unwrap().batches, vec!["5"]);
        let err = host.call("missing", "{}").unwrap_err();
        assert!(err.to_string().contains("JS"), "{err}");
    }
}
