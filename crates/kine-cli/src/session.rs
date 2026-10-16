//! Сессия сцены: живой JS, дерево, сценарий и движение. JS вызывается только на командах
//! сценария, не на кадрах. Время сессии идёт вперёд; запрос момента раньше текущего
//! перезапускает сессию с начала, поэтому кадр зависит только от момента, а не от пути к нему.

use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use kine_core::look::{Rgba, TextAlign, TextCase, TextLook};
use kine_core::diag::Level;
use kine_core::motion::Clock;
use kine_core::scene::{Cue, CueCmd};
use kine_core::text::{RunSpec, TextBlock, TextSource};
use kine_core::{EvalOptions, Frame, Hints, MotionState, Scene, TextEngine, evaluate_at};
use kine_js::{JsException, JsHost, LogLine, RunOptions};

use crate::sourcemap::SourceMap;
use crate::time::{self, Offset};
use crate::{Bundle, Failure};

/// Время стадий построения кадра.
#[derive(Debug, Clone, Copy, Default)]
pub struct StageTimes {
    /// Команды сценария и вызовы JS до момента кадра.
    pub script: Duration,
    /// Значения движения в момент кадра.
    pub motion: Duration,
    /// Раскладка, текст, `must` и `else`.
    pub layout: Duration,
}

pub struct Session {
    js: JsHost,
    /// Из чего сессия открыта: для перезапуска при перемотке назад.
    bundle: Bundle,
    opts: RunOptions,
    extra: Vec<CueArg>,
    pub scene: Scene,
    engine: TextEngine,
    pub motion: MotionState,
    cues: Vec<Cue>,
    next: usize,
    pub logs: Vec<LogLine>,
    pub fps: f64,
    now: f64,
    /// Конец всех `in` первого play; запоминается, когда этот play обработан.
    settled: Option<f64>,
    /// Подсказки `else` dev-режима, посчитанные на прошлых кадрах.
    hints: Hints,
    /// Подписывать события и команды в часах кадра: они нужны дампу, но не `check` и видео.
    pub labels: bool,
}

impl Session {
    /// Запускает бандл сцены. `extra` — команды из `--cue`. Без сценария сцена получает play в 0.
    pub fn open(bundle: &Bundle, opts: &RunOptions, extra: &[CueArg]) -> Result<Session> {
        let (js, first) = JsHost::start(&bundle.code, opts).map_err(|e| map_js(bundle.map.as_ref(), e))?;
        let mut scene = Scene::default();
        for batch in &first.batches {
            scene.apply_batch(batch)?;
        }
        if !scene.props_errors.is_empty() {
            return Err(Failure::Props(scene.props_errors).into());
        }
        let fps = scene.stage.as_ref().map(|s| s.fps as f64).context("в сцене нет stage: бандл должен вызвать mount(scene)")?;
        if fps <= 0.0 {
            bail!("fps сцены должен быть больше 0");
        }
        let engine = TextEngine::for_scene(&scene);
        let mut motion = MotionState::default();
        motion.observe(&scene, 0.0);

        let from_flags = extra.iter().map(|c| Cue { t: c.at.to_seconds(fps), cmd: c.cmd.clone(), target: c.target.clone() });
        let mut cues: Vec<Cue> = scene.cues.iter().cloned().chain(from_flags).collect();
        // Без play в сценарии сцена выходит в эфир в момент 0: `--cue "2 stop"` — это «показать и снять».
        if !cues.iter().any(|c| c.cmd == CueCmd::Play) {
            cues.insert(0, Cue { t: 0.0, cmd: CueCmd::Play, target: None });
        }
        for cue in &mut cues {
            if let Some(target) = &cue.target
                && MotionState::roots_of(&scene, target).is_empty() {
                    return Err(crate::usage(format!("{}: в сцене нет экземпляров {target}", cue.label())));
                }
            // Команды применяются на границе кадра.
            cue.t = snap(cue.t, fps);
        }
        cues.sort_by(|a, b| a.t.total_cmp(&b.t));
        Ok(Session {
            js,
            bundle: bundle.clone(),
            opts: opts.clone(),
            extra: extra.to_vec(),
            scene,
            engine,
            motion,
            cues,
            next: 0,
            logs: first.logs,
            fps,
            now: 0.0,
            settled: None,
            hints: Hints::default(),
            labels: true,
        })
    }

    pub fn cues(&self) -> &[Cue] {
        &self.cues
    }

    /// Время, до которого сессия уже дошла.
    pub fn now(&self) -> f64 {
        self.now
    }

    /// Номер ближайшего кадра. Одно правило округления для дампа, `check`, раскадровки и видео.
    pub fn frame_index(&self, t: f64) -> u64 {
        (t * self.fps).round().max(0.0) as u64
    }

    /// Момент кадра `n`.
    pub fn frame_time(&self, n: u64) -> f64 {
        n as f64 / self.fps
    }

    /// Номера кадров диапазона, обе границы включительно.
    pub fn frames(&self, from: f64, to: f64) -> std::ops::RangeInclusive<u64> {
        self.frame_index(from)..=self.frame_index(to)
    }

    /// Ближайшая граница кадра.
    pub fn nearest_frame(&self, t: f64) -> f64 {
        self.frame_time(self.frame_index(t))
    }

    /// Ближайшая граница кадра не раньше `t`.
    pub fn snap(&self, t: f64) -> f64 {
        snap(t, self.fps)
    }

    /// Обрабатывает команды и события движения до момента `t` включительно.
    pub fn advance(&mut self, t: f64) -> Result<()> {
        if t + 1e-9 < self.now {
            let labels = self.labels;
            *self = Session::open(&self.bundle, &self.opts, &self.extra)?;
            self.labels = labels;
        }
        while let Some(cue) = self.cues.get(self.next).filter(|c| c.t <= t).cloned() {
            self.next += 1;
            self.motion.advance_to(cue.t);
            self.motion.log.push(cue.label());
            let roots = cue.target.as_deref().map(|c| MotionState::roots_of(&self.scene, c));
            match &cue.cmd {
                CueCmd::Play => {
                    self.motion.play(&self.scene, roots.as_deref(), cue.t);
                    if self.settled.is_none() {
                        self.settled = Some(self.motion.settled_at.unwrap_or(cue.t));
                    }
                }
                CueCmd::Stop => self.motion.stop(&self.scene, roots.as_deref(), cue.t),
                CueCmd::Next => self.motion.next(roots.as_deref(), cue.t),
                CueCmd::Update { props } => {
                    let arg = serde_json::json!({ "target": cue.target, "props": props });
                    self.call_js("__kine_update", &arg.to_string(), &cue)?;
                }
                CueCmd::Invoke { command, props } => {
                    let arg = serde_json::json!({ "target": cue.target, "command": command, "params": props });
                    self.call_js("__kine_invoke", &arg.to_string(), &cue)?;
                }
            }
        }
        self.motion.advance_to(t);
        self.now = t;
        Ok(())
    }

    /// Вызов JS на команде сценария: пакеты применяются, изменения анимируются, а команды
    /// движения из `ctx` исполняются в тот же момент.
    fn call_js(&mut self, name: &str, arg: &str, cue: &Cue) -> Result<()> {
        let out = self.js.call(name, arg).map_err(|e| map_js(self.bundle.map.as_ref(), e)).with_context(|| format!("команда {}", cue.label()))?;
        self.logs.extend(out.logs);
        for batch in &out.batches {
            self.scene.apply_batch(batch)?;
        }
        self.motion.observe(&self.scene, cue.t);
        for m in std::mem::take(&mut self.scene.motion_cmds) {
            if let Err(e) = self.motion.instance_cmd(&self.scene, &m.inst, &m.action, cue.t) {
                self.motion.diagnostics.push(Level::Error, "command.motion", &m.inst, None, format!("{}: {e}", cue.label()), None);
            }
        }
        Ok(())
    }

    /// Момент, когда воспроизведение впервые проходит маркер таймлайна.
    pub fn marker_time(&mut self, name: &str) -> Result<f64> {
        const LIMIT: f64 = 600.0;
        loop {
            if let Some(e) = self.motion.events_log.iter().find(|e| e.kind == "marker" && e.name == name) {
                return Ok(e.t);
            }
            let cue = self.cues.get(self.next).map(|c| c.t);
            let next = match (cue, self.motion.next_event_time()) {
                (Some(a), Some(b)) => a.min(b),
                (a, b) => match a.or(b) {
                    Some(t) => t,
                    None => return Err(crate::usage(format!("маркер {name:?} не встретился: воспроизведение до него не доходит"))),
                },
            };
            if next > LIMIT {
                return Err(crate::usage(format!("маркер {name:?} не встретился за {LIMIT} с")));
            }
            self.advance(next.max(self.now))?;
        }
    }

    /// Кадр в момент `t`: раскладка, `must`/`else` и движение.
    pub fn frame_at(&mut self, t: f64, eval: EvalOptions) -> Result<Frame> {
        self.frame_timed(t, eval).map(|(frame, _)| frame)
    }

    /// Кадр и время его стадий — для `kine perf`.
    pub fn frame_timed(&mut self, t: f64, eval: EvalOptions) -> Result<(Frame, StageTimes)> {
        let start = Instant::now();
        self.advance(t)?;
        let script = start.elapsed();
        let mut anim = self.motion.values(t);
        let (log, events) = if self.labels {
            (self.motion.log.clone(), self.motion.events_log.iter().filter(|e| e.t <= t + 1e-9).map(|e| e.label()).collect())
        } else {
            Default::default()
        };
        anim.clock = Some(Clock { t, frame: self.frame_index(t), scene: self.motion.scene_phase(), log, events });
        let motion = start.elapsed() - script;
        let frame = evaluate_at(&self.scene, &mut self.engine, eval, &anim, &mut self.hints)?;
        let layout = start.elapsed() - script - motion;
        Ok((frame, StageTimes { script, motion, layout }))
    }

    /// Кадр «в эфире»: момент, когда закончились все `in` первого play (на границе кадра).
    pub fn settled_time(&mut self) -> Result<f64> {
        if self.settled.is_none() {
            let Some(play) = self.cues.iter().find(|c| c.cmd == CueCmd::Play).map(|c| c.t) else { return Ok(0.0) };
            self.advance(play)?;
        }
        Ok(self.snap(self.settled.unwrap_or(0.0)))
    }

    /// Конец сценария: последняя команда и всё, что она запустила.
    pub fn end_time(&mut self) -> Result<f64> {
        let last = self.cues.last().map_or(0.0, |c| c.t);
        self.advance(last.max(self.now))?;
        let settled = self.motion.settled_at.unwrap_or(0.0);
        Ok(self.snap(self.motion.horizon.max(last).max(settled)))
    }

    /// Подпись для раскадровки шрифтом сцены.
    pub fn label(&mut self, text: &str, size: f32, color: Rgba) -> Option<TextBlock> {
        let family = self.engine.fonts.iter().flat_map(|f| f.families.first()).next().cloned();
        if family.is_none() && !self.engine.system_fonts {
            return None;
        }
        let look = TextLook {
            family,
            size,
            weight: 400.0,
            color,
            line_height: None,
            tracking: 0.0,
            align: TextAlign::Start,
            case: TextCase::None,
        };
        let mut block = self.engine.build(TextSource { runs: vec![RunSpec { text: text.into(), style: None }], look, fit: None });
        block.fit_to(&mut self.engine, None);
        Some(block)
    }
}

/// Стек исключения JS через sourcemap: строки TSX вместо строк бандла.
pub(crate) fn map_js(map: Option<&SourceMap>, e: anyhow::Error) -> anyhow::Error {
    match (map, e.downcast::<JsException>()) {
        (Some(map), Ok(mut ex)) => {
            map.map_exception(&mut ex);
            anyhow::Error::new(ex)
        }
        (_, Ok(ex)) => anyhow::Error::new(ex),
        (_, Err(e)) => e,
    }
}

fn snap(t: f64, fps: f64) -> f64 {
    ((t * fps - 1e-6).ceil().max(0.0)) / fps
}

/// Команда из `--cue`: время ещё в секундах или кадрах, fps знает только сессия.
#[derive(Debug, Clone, PartialEq)]
pub struct CueArg {
    pub at: Offset,
    /// play, stop или next: команды с параметрами задаются в `*.cues.ts`.
    pub cmd: CueCmd,
    /// Компонент: команда только для его экземпляров.
    pub target: Option<String>,
}

/// Короткая форма сценария: `--cue "0 play; 125f stop"`. Время — по общей грамматике.
pub fn parse_cues(spec: &str) -> Result<Vec<CueArg>> {
    let mut out = Vec::new();
    for part in spec.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let mut words = part.split_whitespace();
        let (Some(t), Some(cmd)) = (words.next(), words.next()) else {
            bail!("--cue: ожидается \"<время> <команда>\", получено {part:?}");
        };
        let at = time::cue_time(t).with_context(|| format!("--cue {part:?}"))?;
        let cmd = match cmd {
            "play" => CueCmd::Play,
            "stop" => CueCmd::Stop,
            "next" => CueCmd::Next,
            "update" | "invoke" => bail!("--cue: {cmd} с параметрами задаётся в *.cues.ts: at(t, '{cmd}', Component, ...)"),
            other => bail!("--cue: команда {other:?} не поддерживается; есть play, stop, next"),
        };
        let target = words.next().map(String::from);
        if let Some(extra) = words.next() {
            bail!("--cue: лишнее {extra:?} в {part:?}; форма: \"<время> <команда> [Компонент]\"");
        }
        out.push(CueArg { at, cmd, target });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_rounds_up_to_frame() {
        assert_eq!(snap(0.0, 50.0), 0.0);
        assert_eq!(snap(0.5, 50.0), 0.5);
        assert_eq!(snap(0.51, 50.0), 0.52);
        assert_eq!(snap(1.0 / 3.0, 30.0), 10.0 / 30.0);
    }

    #[test]
    fn cue_shorthand() {
        let cues = parse_cues("0 play; 125f stop").unwrap();
        assert_eq!(
            cues,
            [CueArg { at: Offset::Seconds(0.0), cmd: CueCmd::Play, target: None }, CueArg { at: Offset::Frames(125), cmd: CueCmd::Stop, target: None }]
        );
        assert_eq!(parse_cues("1 next Scoreboard").unwrap()[0].target.as_deref(), Some("Scoreboard"));
        assert!(parse_cues("1 pause").is_err());
        assert!(parse_cues("x play").is_err());
    }
}
