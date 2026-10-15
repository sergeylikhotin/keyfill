//! Сессия сцены: живой JS, дерево, сценарий и движение. Кадры запрашиваются по
//! возрастанию времени; JS вызывается только на командах сценария, не на кадрах.

use anyhow::{Context as _, Result, bail};
use kine_core::look::{Rgba, TextAlign, TextCase, TextLook};
use kine_core::motion::Clock;
use kine_core::scene::Cue;
use kine_core::text::{RunSpec, TextBlock, TextSource};
use kine_core::{EvalOptions, Frame, MotionState, Scene, TextEngine, evaluate_at};
use kine_js::{JsHost, LogLine, RunOptions};

use crate::time::{self, Offset};

const COMMANDS: &[&str] = &["play", "stop", "update"];

pub struct Session {
    js: JsHost,
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
}

impl Session {
    /// Запускает бандл сцены. `extra` — команды из `--cue`. Без сценария сцена получает play в 0.
    pub fn open(source: &str, opts: &RunOptions, extra: &[CueArg]) -> Result<Session> {
        let (js, first) = JsHost::start(source, opts)?;
        let mut scene = Scene::default();
        for batch in &first.batches {
            scene.apply_batch(batch)?;
        }
        let fps = scene.stage.as_ref().map(|s| s.fps as f64).context("в сцене нет stage: бандл должен вызвать mount(scene)")?;
        if fps <= 0.0 {
            bail!("fps сцены должен быть больше 0");
        }
        let engine = TextEngine::for_scene(&scene);
        let mut motion = MotionState::default();
        motion.observe(&scene, 0.0);

        let extra = extra.iter().map(|c| Cue { t: c.at.to_seconds(fps), cmd: c.cmd.clone(), target: None, props: None });
        let mut cues: Vec<Cue> = scene.cues.iter().cloned().chain(extra).collect();
        // Без play в сценарии сцена выходит в эфир в момент 0: `--cue "2 stop"` — это «показать и снять».
        if !cues.iter().any(|c| c.cmd == "play") {
            cues.insert(0, Cue { t: 0.0, cmd: "play".into(), target: None, props: None });
        }
        for cue in &mut cues {
            if !COMMANDS.contains(&cue.cmd.as_str()) {
                let err = anyhow::anyhow!("команда сценария {:?} не поддерживается; есть {}", cue.cmd, COMMANDS.join(", "));
                return Err(err.context(crate::Failure::Usage));
            }
            // Команды применяются на границе кадра.
            cue.t = snap(cue.t, fps);
        }
        cues.sort_by(|a, b| a.t.total_cmp(&b.t));
        Ok(Session { js, scene, engine, motion, cues, next: 0, logs: first.logs, fps, now: 0.0, settled: None })
    }

    pub fn cues(&self) -> &[Cue] {
        &self.cues
    }

    /// Ближайшая граница кадра не раньше `t`.
    pub fn snap(&self, t: f64) -> f64 {
        snap(t, self.fps)
    }

    /// Обрабатывает команды и события движения до момента `t` включительно.
    pub fn advance(&mut self, t: f64) -> Result<()> {
        if t + 1e-9 < self.now {
            bail!("время сессии идёт только вперёд: запрошено {t}, уже {}", self.now);
        }
        while let Some(cue) = self.cues.get(self.next).filter(|c| c.t <= t).cloned() {
            self.next += 1;
            self.motion.advance_to(cue.t);
            match cue.cmd.as_str() {
                "play" => {
                    self.motion.play(cue.t);
                    if self.settled.is_none() {
                        self.settled = Some(self.motion.settled_at.unwrap_or(cue.t));
                    }
                }
                "stop" => self.motion.stop(cue.t),
                "update" => {
                    let arg = serde_json::json!({ "target": cue.target, "props": cue.props });
                    let out = self.js.call("__kine_update", &arg.to_string()).with_context(|| format!("команда {}", cue.label()))?;
                    self.logs.extend(out.logs);
                    for batch in &out.batches {
                        self.scene.apply_batch(batch)?;
                    }
                    self.motion.log.push(cue.label());
                    self.motion.observe(&self.scene, cue.t);
                }
                _ => unreachable!("команды проверены в open"),
            }
        }
        self.motion.advance_to(t);
        self.now = t;
        Ok(())
    }

    /// Кадр в момент `t`: раскладка, `must`/`else` и движение.
    pub fn frame_at(&mut self, t: f64, eval: EvalOptions) -> Result<Frame> {
        self.advance(t)?;
        let mut anim = self.motion.values(t);
        anim.clock = Some(Clock {
            t,
            frame: (t * self.fps).round() as u64,
            scene: self.motion.scene_phase().label(),
            log: self.motion.log.clone(),
        });
        evaluate_at(&self.scene, &mut self.engine, eval, &anim)
    }

    /// Кадр «в эфире»: момент, когда закончились все `in` первого play (на границе кадра).
    pub fn settled_time(&mut self) -> Result<f64> {
        if self.settled.is_none() {
            let Some(play) = self.cues.iter().find(|c| c.cmd == "play").map(|c| c.t) else { return Ok(0.0) };
            self.advance(play)?;
        }
        Ok(self.snap(self.settled.unwrap_or(0.0)))
    }

    /// Конец сценария: последняя команда и всё, что она запустила.
    pub fn end_time(&mut self) -> Result<f64> {
        let last = self.cues.last().map_or(0.0, |c| c.t);
        self.advance(last)?;
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

fn snap(t: f64, fps: f64) -> f64 {
    ((t * fps - 1e-6).ceil().max(0.0)) / fps
}

/// Команда из `--cue`: время ещё в секундах или кадрах, fps знает только сессия.
#[derive(Debug, Clone, PartialEq)]
pub struct CueArg {
    pub at: Offset,
    pub cmd: String,
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
        match cmd {
            "play" | "stop" => {}
            "update" => bail!("--cue: update с пропсами задаётся в *.cues.ts: at(t, 'update', Component, {{ ... }})"),
            other => bail!("--cue: команда {other:?} не поддерживается; есть play, stop"),
        }
        if let Some(extra) = words.next() {
            bail!("--cue: лишнее {extra:?} в {part:?}; адресные команды появятся с таймлайнами (M2b)");
        }
        out.push(CueArg { at, cmd: cmd.into() });
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
        assert_eq!(cues, [CueArg { at: Offset::Seconds(0.0), cmd: "play".into() }, CueArg { at: Offset::Frames(125), cmd: "stop".into() }]);
        assert!(parse_cues("1 next").is_err());
        assert!(parse_cues("x play").is_err());
    }
}
