//! QuickJS-хост: исполняет бандл сцены и собирает пакеты команд, которые JS шлёт в ядро.
//!
//! JS не видит реальное время и настоящую случайность: `Date.now`, `performance.now`
//! и `Math.random` подменены до запуска бандла, таймеры запрещены до появления часов kine (M2).

use std::cell::RefCell;
use std::rc::Rc;

use anyhow::{Result, anyhow};
use rquickjs::{CaughtError, CatchResultExt, Context, Function, Runtime};

#[derive(Debug, Clone)]
pub struct RunOptions {
    /// Сид для `Math.random`.
    pub seed: u32,
    /// Значение `Date.now()` и `performance.now()` в миллисекундах.
    pub now_ms: f64,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self { seed: 1, now_ms: 0.0 }
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
  const noTimers = () => {
    throw new Error('kine: таймеры идут от часов kine и появятся в M2; используй time() или presence/timeline');
  };
  globalThis.setTimeout = noTimers;
  globalThis.setInterval = noTimers;
  globalThis.requestAnimationFrame = noTimers;
})();
"#;

/// Исполняет бандл сцены и возвращает всё, что он отправил в ядро.
pub fn run_bundle(source: &str, opts: &RunOptions) -> Result<RunOutput> {
    let rt = Runtime::new()?;
    let ctx = Context::full(&rt)?;
    let out = Rc::new(RefCell::new(RunOutput::default()));

    let result: Result<()> = ctx.with(|ctx| {
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
            .replace("__KINE_NOW__", &format!("{:?}", opts.now_ms));
        ctx.eval::<(), _>(prelude).catch(&ctx).map_err(js_error)?;
        ctx.eval::<(), _>(source).catch(&ctx).map_err(js_error)?;
        Ok(())
    });
    result?;

    // Замыкания в рантайме держат свои клоны Rc до его уничтожения, поэтому забираем содержимое.
    Ok(std::mem::take(&mut *out.borrow_mut()))
}

fn js_error(e: CaughtError<'_>) -> anyhow::Error {
    match e {
        CaughtError::Exception(ex) => {
            let message = ex.message().unwrap_or_default();
            // KineError уже несёт место в TSX; стек бандла агенту ничего не добавляет.
            let name: Option<String> = ex.get("name").ok();
            if name.as_deref() == Some("KineError") {
                return anyhow!("{message}");
            }
            let stack = ex.stack().unwrap_or_default();
            anyhow!("JS: {message}\n{stack}")
        }
        other => anyhow!("JS: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_batches_and_logs() {
        let out = run_bundle("console.log('hi', 1); __kine_emit('[1]'); __kine_emit('[2]');", &RunOptions::default()).unwrap();
        assert_eq!(out.batches, vec!["[1]", "[2]"]);
        assert_eq!(out.logs, vec![LogLine { level: "log".into(), message: "hi 1".into() }]);
    }

    #[test]
    fn time_and_random_are_deterministic() {
        let src = "__kine_emit(JSON.stringify([Date.now(), performance.now(), Math.random(), Math.random()]));";
        let opts = RunOptions { seed: 42, now_ms: 1500.0 };
        let a = run_bundle(src, &opts).unwrap();
        let b = run_bundle(src, &opts).unwrap();
        assert_eq!(a.batches, b.batches);
        assert!(a.batches[0].starts_with("[1500,1500,"));
    }

    #[test]
    fn timers_are_rejected_with_hint() {
        let err = run_bundle("setTimeout(() => {}, 10)", &RunOptions::default()).unwrap_err();
        assert!(err.to_string().contains("M2"), "{err}");
    }
}
