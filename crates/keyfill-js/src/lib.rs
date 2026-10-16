//! QuickJS-хост: исполняет бандл сцены и собирает пакеты команд, которые JS шлёт в ядро.
//!
//! Хост живёт всю сессию: после монтирования ядро вызывает JS на командах сценария
//! (`update`), а не на каждом кадре. JS не видит реальное время и настоящую случайность:
//! `Date.now`, `performance.now` и `Math.random` подменены до запуска бандла, таймеров нет —
//! время ведёт ядро.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
    /// Сколько JS может исполняться за один вызов (монтирование, команда сценария): бесконечный
    /// цикл в сцене прерывается исключением, а не вешает CLI и просмотрщик.
    pub budget: Duration,
    /// Что бандл делает с файлом точки входа.
    pub entry: EntryMode,
}

/// Что делает бандл с файлом точки входа (`__keyfill_main` в `@keyfill/core`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EntryMode {
    /// Сцена, сценарий или `keyfill.config.ts`: по тому, что экспортирует файл.
    #[default]
    Default,
    /// Файл тестов: отдать список случаев, ничего не монтируя.
    Tests,
    /// Файл тестов: смонтировать сцену случая с его пропсами.
    Case(usize),
}

impl Default for RunOptions {
    fn default() -> Self {
        Self { seed: 1, now_ms: 0.0, props: None, budget: Duration::from_secs(10), entry: EntryMode::Default }
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
    log: (...a) => __keyfill_log('log', fmt(a)),
    info: (...a) => __keyfill_log('info', fmt(a)),
    warn: (...a) => __keyfill_log('warn', fmt(a)),
    error: (...a) => __keyfill_log('error', fmt(a)),
  };
  let s = __KEYFILL_SEED__ >>> 0;
  Math.random = () => {
    s = (Math.imul(s, 1664525) + 1013904223) >>> 0;
    return s / 4294967296;
  };
  // Часы и часовой пояс сцены: «сейчас» — __KEYFILL_NOW__, пояс — UTC на любой машине.
  // Локальные методы Date становятся UTC-методами, конструктор без аргументов даёт «сейчас».
  const NOW = __KEYFILL_NOW__;
  const RealDate = Date;
  const P = RealDate.prototype;
  const realOffset = P.getTimezoneOffset;
  for (const n of ['FullYear', 'Month', 'Date', 'Day', 'Hours', 'Minutes', 'Seconds', 'Milliseconds']) {
    P['get' + n] = P['getUTC' + n];
    if (n !== 'Day') P['set' + n] = P['setUTC' + n];
  }
  P.getTimezoneOffset = () => 0;
  const utc = function () { return P.toUTCString.call(this); };
  P.toString = P.toDateString = P.toTimeString = utc;
  P.toLocaleString = P.toLocaleDateString = P.toLocaleTimeString = utc;
  // Строка без пояса разбирается как местное время машины — переводим её в UTC.
  const parse = (s) => {
    const t = RealDate.parse(s);
    const str = String(s).trim();
    if (Number.isNaN(t) || /(Z|[+-]\d\d:?\d\d|GMT|UTC)$/i.test(str) || /^\d{4}(-\d\d){0,2}$/.test(str)) return t;
    return t - realOffset.call(new RealDate(t)) * 60000;
  };
  function KeyfillDate(...a) {
    if (!new.target) return P.toUTCString.call(new RealDate(NOW));
    const v = a.length === 0 ? NOW : a.length === 1 ? (typeof a[0] === 'string' ? parse(a[0]) : a[0]) : RealDate.UTC(...a);
    return Reflect.construct(RealDate, [v], new.target);
  }
  KeyfillDate.prototype = P;
  P.constructor = KeyfillDate;
  KeyfillDate.now = () => NOW;
  KeyfillDate.parse = parse;
  KeyfillDate.UTC = RealDate.UTC;
  globalThis.Date = KeyfillDate;
  globalThis.performance = { now: () => NOW };
  globalThis.__keyfill_props = __KEYFILL_PROPS__;
  globalThis.__keyfill_entry = __KEYFILL_ENTRY__;
  const noTimers = () => {
    throw new Error('keyfill: таймеров нет, время ведёт ядро: движение — presence/transition/timeline, смена данных — команды сценария (cues)');
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
    watchdog: Watchdog,
}

/// Сторож бюджета: перед каждым вызовом JS заводится срок, обработчик прерываний QuickJS
/// останавливает исполнение после него.
#[derive(Clone)]
struct Watchdog {
    deadline: Arc<Mutex<Instant>>,
    tripped: Arc<AtomicBool>,
    budget: Duration,
}

impl Watchdog {
    fn new(rt: &Runtime, budget: Duration) -> Watchdog {
        let w = Watchdog { deadline: Arc::new(Mutex::new(Instant::now() + budget)), tripped: Arc::default(), budget };
        let (deadline, tripped) = (w.deadline.clone(), w.tripped.clone());
        rt.set_interrupt_handler(Some(Box::new(move || {
            let over = Instant::now() > *deadline.lock().unwrap_or_else(|e| e.into_inner());
            if over {
                tripped.store(true, Ordering::Relaxed);
            }
            over
        })));
        w
    }

    fn arm(&self) {
        *self.deadline.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now() + self.budget;
        self.tripped.store(false, Ordering::Relaxed);
    }

    /// Исключение «interrupted» после срока — понятная ошибка с подсказкой.
    fn explain(&self, e: anyhow::Error) -> anyhow::Error {
        if !self.tripped.load(Ordering::Relaxed) {
            return e;
        }
        let mut ex = e.downcast::<JsException>().unwrap_or_else(|e| JsException { message: e.to_string(), keyfill: false, source: None, stack: Vec::new(), hidden: 0 });
        ex.message = format!("JS исполнялся дольше {} с и прерван: бесконечный цикл?", self.budget.as_secs_f64());
        anyhow::Error::new(ex)
    }
}

impl JsHost {
    /// Запускает бандл; возвращает хост и то, что сцена отправила при монтировании.
    pub fn start(source: &str, opts: &RunOptions) -> Result<(JsHost, RunOutput)> {
        let rt = Runtime::new()?;
        let watchdog = Watchdog::new(&rt, opts.budget);
        let ctx = Context::full(&rt)?;
        let out = Rc::new(RefCell::new(RunOutput::default()));
        watchdog.arm();
        init(&ctx, &out, source, opts).map_err(|e| watchdog.explain(e))?;
        let first = std::mem::take(&mut *out.borrow_mut());
        Ok((JsHost { ctx, _rt: rt, out, watchdog }, first))
    }

    /// Вызывает глобальную функцию сцены с JSON-аргументом и возвращает отправленные пакеты.
    pub fn call(&mut self, name: &str, arg_json: &str) -> Result<RunOutput> {
        self.watchdog.arm();
        let result: Result<()> = self.ctx.with(|ctx| {
            let f: Function = ctx.globals().get(name).catch(&ctx).map_err(js_error)?;
            f.call::<_, ()>((arg_json.to_string(),)).catch(&ctx).map_err(js_error)?;
            Ok(())
        });
        result.map_err(|e| self.watchdog.explain(e))?;
        Ok(std::mem::take(&mut *self.out.borrow_mut()))
    }
}

fn init(ctx: &Context, out: &Rc<RefCell<RunOutput>>, source: &str, opts: &RunOptions) -> Result<()> {
    ctx.with(|ctx| {
        let globals = ctx.globals();

        let sink = out.clone();
        globals.set(
            "__keyfill_emit",
            Function::new(ctx.clone(), move |json: String| sink.borrow_mut().batches.push(json))?,
        )?;
        let sink = out.clone();
        globals.set(
            "__keyfill_log",
            Function::new(ctx.clone(), move |level: String, message: String| {
                sink.borrow_mut().logs.push(LogLine { level, message })
            })?,
        )?;

        let prelude = PRELUDE
            .replace("__KEYFILL_SEED__", &opts.seed.to_string())
            .replace("__KEYFILL_NOW__", &format!("{:?}", opts.now_ms))
            .replace("__KEYFILL_PROPS__", opts.props.as_deref().unwrap_or("null"))
            .replace(
                "__KEYFILL_ENTRY__",
                &match opts.entry {
                    EntryMode::Default => "null".to_string(),
                    EntryMode::Tests => "'tests'".to_string(),
                    EntryMode::Case(i) => i.to_string(),
                },
            );
        ctx.eval::<(), _>(prelude).catch(&ctx).map_err(js_error)?;
        ctx.eval::<(), _>(source).catch(&ctx).map_err(js_error)?;
        Ok(())
    })
}

/// Исключение в JS сцены. Отдельный тип, чтобы CLI отличал его от сбоя самого keyfill;
/// стек разобран на кадры здесь, чтобы sourcemap и диагностика работали с полями, а не с текстом.
#[derive(Debug, Clone, PartialEq)]
pub struct JsException {
    pub message: String,
    /// Ошибка API keyfill (`KeyfillError`): место в TSX известно, стек бандла не нужен.
    pub keyfill: bool,
    /// Место в исходнике: у `KeyfillError` — из dev-JSX, иначе первый кадр стека после sourcemap.
    pub source: Option<String>,
    pub stack: Vec<Frame>,
    /// Сколько кадров скрыто как внутренности `@keyfill/core`.
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
        if self.keyfill {
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
            write!(f, "\n    … ещё {} в @keyfill/core", self.hidden)?;
        }
        Ok(())
    }
}

impl std::error::Error for JsException {}

fn js_error(e: CaughtError<'_>) -> anyhow::Error {
    let ex = match e {
        CaughtError::Exception(ex) => {
            let name: Option<String> = ex.get("name").ok();
            let keyfill = name.as_deref() == Some("KeyfillError");
            JsException {
                message: ex.message().unwrap_or_default(),
                keyfill,
                source: if keyfill { ex.get::<_, Option<String>>("src").ok().flatten() } else { None },
                stack: if keyfill { Vec::new() } else { ex.stack().unwrap_or_default().lines().filter_map(Frame::parse).collect() },
                hidden: 0,
            }
        }
        other => JsException { message: other.to_string(), keyfill: false, source: None, stack: Vec::new(), hidden: 0 },
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
        let out = run_bundle("console.log('hi', 1); __keyfill_emit('[1]'); __keyfill_emit('[2]');", &RunOptions::default()).unwrap();
        assert_eq!(out.batches, vec!["[1]", "[2]"]);
        assert_eq!(out.logs, vec![LogLine { level: "log".into(), message: "hi 1".into() }]);
    }

    #[test]
    fn time_and_random_are_deterministic() {
        let src = "__keyfill_emit(JSON.stringify([Date.now(), performance.now(), Math.random(), Math.random()]));";
        let opts = RunOptions { seed: 42, now_ms: 1500.0, ..Default::default() };
        let a = run_bundle(src, &opts).unwrap();
        let b = run_bundle(src, &opts).unwrap();
        assert_eq!(a.batches, b.batches);
        assert!(a.batches[0].starts_with("[1500,1500,"));
    }

    #[test]
    fn date_is_fixed_and_utc() {
        let src = "__keyfill_emit(JSON.stringify([new Date().toISOString(), new Date(2020, 0, 2, 3).getHours(), new Date(0).getTimezoneOffset(), Date(), new Date('2020-01-02T03:04').getHours()]));";
        let out = run_bundle(src, &RunOptions { now_ms: 1_000.0, ..Default::default() }).unwrap();
        assert_eq!(out.batches[0], r#"["1970-01-01T00:00:01.000Z",3,0,"Thu, 01 Jan 1970 00:00:01 GMT",3]"#);
    }

    #[test]
    fn endless_loop_is_interrupted() {
        let opts = RunOptions { budget: Duration::from_millis(100), ..Default::default() };
        let err = run_bundle("while (true) {}", &opts).unwrap_err();
        assert!(err.to_string().contains("бесконечный цикл"), "{err}");
    }

    #[test]
    fn timers_are_rejected_with_hint() {
        let err = run_bundle("setTimeout(() => {}, 10)", &RunOptions::default()).unwrap_err();
        assert!(err.to_string().contains("время ведёт ядро"), "{err}");
    }

    #[test]
    fn host_keeps_state_between_calls() {
        let src = "let n = 0; globalThis.bump = (json) => { n += JSON.parse(json).by; __keyfill_emit(String(n)); };";
        let (mut host, first) = JsHost::start(src, &RunOptions::default()).unwrap();
        assert!(first.batches.is_empty());
        assert_eq!(host.call("bump", r#"{"by":2}"#).unwrap().batches, vec!["2"]);
        assert_eq!(host.call("bump", r#"{"by":3}"#).unwrap().batches, vec!["5"]);
        let err = host.call("missing", "{}").unwrap_err();
        assert!(err.to_string().contains("JS"), "{err}");
    }
}
