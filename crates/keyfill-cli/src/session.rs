//! Сессия сцены: живой JS, дерево, сценарий и движение. JS вызывается только на командах
//! сценария, не на кадрах. Время сессии идёт вперёд; запрос момента раньше текущего
//! перезапускает сессию с начала, поэтому кадр зависит только от момента, а не от пути к нему.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use keyfill_core::look::{Rgba, TextAlign, TextCase, TextLook};
use keyfill_core::diag::Level;
use keyfill_core::motion::Clock;
use keyfill_core::scene::{Cue, CueCmd};
use keyfill_core::text::{RunSpec, TextBlock, TextSource};
use keyfill_core::layout::Overrides;
use keyfill_core::motion::GeoMap;
use keyfill_core::scene::ChannelDef;
use keyfill_core::transition::{self, Act, PlanStep, StepKind, TransitionPlan};
use keyfill_core::Rect;
use keyfill_core::{EvalOptions, Frame, Hints, MotionState, Scene, TextEngine, build_frame_with, evaluate_at, geometry};
use keyfill_js::{JsException, JsHost, LogLine, RunOptions};

use crate::channel::{Ack, AckStatus, Batch, Command, Entry, Inst, InstState, Layer, State};
use crate::sourcemap::SourceMap;
use crate::time::{self, Offset};
use crate::{Bundle, Failure};
use keyfill_core::motion::MotionEvent;
use keyfill_core::scene::LayerReply;

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
    /// Канал: слои с экземплярами; `None` — обычная сцена.
    pub channel: Option<State>,
    /// Пачки команд канала: из журнала и принятые `command` в этой сессии.
    batches: Vec<Batch>,
    next_batch: usize,
    /// Отпущенные корни (элементы `For`, экземпляры после stop): удаляются, когда кончится уход.
    releasing: Vec<u32>,
    /// Уходящие экземпляры состояний: живы в JS, пока не кончится уход; их может развернуть
    /// следующее состояние.
    parked: Vec<u32>,
    /// Шаги перехода, которые ждут своего момента, по времени.
    pending: Vec<Pending>,
    seq: u64,
    /// Расписание последнего перехода — для дампа.
    plan: Option<TransitionPlan>,
    /// Пачки, принятые `command` с прошлого `take_recorded`: их дописывает в журнал `serve`.
    recorded: Vec<Batch>,
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
    /// Бандл канала (`defineChannel`) играет пачки команд `batches` из журнала.
    pub fn open(bundle: &Bundle, opts: &RunOptions, extra: &[CueArg], batches: &[Batch]) -> Result<Session> {
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
        let channel = scene.channel.as_ref().map(|c| State {
            managed: c.layers.values().copied().collect(),
            data: c.data.clone(),
            ..State::default()
        });
        if channel.is_some() {
            if !extra.is_empty() {
                return Err(crate::usage("--cue — для сцен; командами канала управляет журнал *.journal.jsonl"));
            }
            motion.start_channel(0.0);
        } else if !batches.is_empty() {
            return Err(crate::usage("журнал команд играет канал (defineChannel), а файл экспортирует сцену"));
        }
        motion.observe(&scene, 0.0);

        let from_flags = extra.iter().map(|c| Cue { t: c.at.to_seconds(fps), cmd: c.cmd.clone(), target: c.target.clone() });
        let mut cues: Vec<Cue> = scene.cues.iter().cloned().chain(from_flags).collect();
        // Без play в сценарии сцена выходит в эфир в момент 0: `--cue "2 stop"` — это «показать и снять».
        // Канал пуст, пока команды не поставят что-то на слои.
        if channel.is_none() && !cues.iter().any(|c| c.cmd == CueCmd::Play) {
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
            channel,
            batches: batches.to_vec(),
            next_batch: 0,
            releasing: Vec::new(),
            parked: Vec::new(),
            pending: Vec::new(),
            seq: 0,
            plan: None,
            recorded: Vec::new(),
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

    /// Пачки команд канала: из журнала и принятые в этой сессии.
    pub fn batches(&self) -> &[Batch] {
        &self.batches
    }

    /// Момент пачки: номер кадра канала.
    fn batch_time(&self, b: &Batch) -> f64 {
        self.frame_time(b.frame)
    }

    /// Ближайшая необработанная команда: сценария или пачка канала.
    fn next_command_time(&self) -> Option<f64> {
        let cue = self.cues.get(self.next).map(|c| c.t);
        let batch = self.batches.get(self.next_batch).map(|b| self.batch_time(b));
        match (cue, batch) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
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
            *self = Session::open(&self.bundle, &self.opts, &self.extra, &self.batches)?;
            self.labels = labels;
        }
        loop {
            let cue = self.cues.get(self.next).filter(|c| c.t <= t).map(|c| c.t);
            let batch = self.batches.get(self.next_batch).map(|b| self.batch_time(b)).filter(|bt| *bt <= t + 1e-9);
            let step = self.pending.first().map(|p| p.at).filter(|at| *at <= t + 1e-9);
            let horizon = [cue, batch, step].into_iter().flatten().reduce(f64::min).unwrap_or(t);
            // Узел ушёл из раскладки до следующей команды: соседи переезжают в этот момент.
            if let Some(et) = self.motion.next_layout_event().filter(|et| *et <= horizon + 1e-9) {
                self.layout_event(et)?;
                continue;
            }
            // Шаг перехода, назначенный раньше, идёт до команды в тот же момент.
            if step.is_some_and(|s| s <= horizon + 1e-9) {
                self.run_pending()?;
                continue;
            }
            self.sweep(horizon)?;
            match (cue, batch) {
                (None, None) => break,
                (Some(c), Some(b)) if b <= c => self.replay_batch()?,
                (None, Some(_)) => self.replay_batch()?,
                _ => self.run_cue()?,
            }
        }
        self.motion.advance_to(t);
        self.sweep(t)?;
        self.now = t;
        Ok(())
    }

    /// Следующая команда сценария сцены.
    fn run_cue(&mut self) -> Result<()> {
        let cue = self.cues[self.next].clone();
        self.next += 1;
        self.motion.advance_to(cue.t);
        self.motion.log.push(cue.label());
        let roots = cue.target.as_deref().map(|c| MotionState::roots_of(&self.scene, c));
        self.relayout(cue.t, |s| s.cue_command(&cue, roots))
    }

    fn cue_command(&mut self, cue: &Cue, roots: Option<Vec<u32>>) -> Result<()> {
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
                self.call_js("__keyfill_update", &arg.to_string(), cue)?;
            }
            CueCmd::Invoke { command, props } => {
                let arg = serde_json::json!({ "target": cue.target, "command": command, "params": props });
                self.call_js("__keyfill_invoke", &arg.to_string(), cue)?;
            }
        }
        Ok(())
    }

    /// Изменение в момент `t`. Если в сцене есть `transition.layout`, раскладка снимается до
    /// и после него, и узлы, у которых изменились место или размер, переезжают. Моменты
    /// снятия — команды и уходы из раскладки, а не кадры: кадр по-прежнему зависит только от
    /// момента, а не от того, какие кадры рендерились до него.
    fn relayout<R>(&mut self, t: f64, change: impl FnOnce(&mut Self) -> Result<R>) -> Result<R> {
        if !self.motion.has_layout() {
            return change(self);
        }
        let before = self.geometry(t)?;
        let out = change(self)?;
        let after = self.geometry(t)?;
        self.motion.relayout(&before, &after, t);
        Ok(out)
    }

    /// Узел уходит из раскладки (конец out, снятие экземпляра) в момент `et`.
    fn layout_event(&mut self, et: f64) -> Result<()> {
        self.motion.advance_before(et);
        let before = self.geometry(et)?;
        self.motion.advance_to(et);
        let after = self.geometry(et)?;
        self.motion.relayout(&before, &after, et);
        self.sweep(et)
    }

    /// Раскладка в момент `t` без анимации геометрии.
    fn geometry(&mut self, t: f64) -> Result<GeoMap> {
        self.geometry_with(t, &[])
    }

    /// То же, но экземпляры `reveal`, загруженные и ещё не показанные, стоят в раскладке.
    fn geometry_with(&mut self, t: f64, reveal: &[u32]) -> Result<GeoMap> {
        let mut anim = self.motion.values(t);
        anim.clear_geo();
        for root in reveal {
            if let Some(nv) = anim.nodes.get_mut(root) {
                nv.gone = None;
                nv.show = None;
            }
        }
        let frame = build_frame_with(&self.scene, &mut self.engine, &Overrides::new(), &anim)?;
        Ok(geometry(&frame))
    }

    /// Следующая пачка из журнала. Отклонённая команда или расхождение с записью — ошибка
    /// диагностики: журнал написан руками с ошибкой или записан на другой версии сцены.
    fn replay_batch(&mut self) -> Result<()> {
        let batch = self.batches[self.next_batch].clone();
        self.next_batch += 1;
        let t = self.batch_time(&batch);
        self.motion.advance_to(t);
        self.sweep(t)?;
        self.relayout(t, |s| s.replay_entries(&batch, t))
    }

    fn replay_entries(&mut self, batch: &Batch, t: f64) -> Result<()> {
        for entry in &batch.cmds {
            let ack = self.run_command(entry, t)?;
            let label = format!("кадр {}: {}", batch.frame, entry.cmd.label());
            match (ack.status, &entry.error) {
                (AckStatus::Ok, None) | (AckStatus::Failed, Some(_)) => {}
                (AckStatus::Rejected, _) => {
                    let msg = format!("{label}: команда отклонена: {}", ack.errors.join("; "));
                    self.motion.diagnostics.push(Level::Error, "channel.rejected", "channel", None, msg, None);
                }
                (status, recorded) => {
                    let was = recorded.as_ref().map_or("исполнилась".to_string(), |e| format!("упала: {e}"));
                    let now = if status == AckStatus::Ok { "исполнилась".to_string() } else { format!("упала: {}", ack.errors.join("; ")) };
                    let msg = format!("{label}: при записи команда {was}, сейчас {now}");
                    self.motion.diagnostics.push(Level::Error, "journal.diverged", "channel", None, msg, None);
                }
            }
        }
        Ok(())
    }

    /// Живая пачка команд в текущий момент сессии (на границе кадра): принятые команды и
    /// упавшие в JS пишутся в журнал сессии, отклонённые — нет.
    pub fn command(&mut self, cmds: Vec<Entry>) -> Result<Vec<Ack>> {
        if self.channel.is_none() {
            return Err(crate::usage("команды слоям — для канала (defineChannel)"));
        }
        let t = self.snap(self.now);
        self.advance(t)?;
        let (acks, kept) = self.relayout(t, |s| {
            let mut acks = Vec::new();
            let mut kept = Vec::new();
            for mut entry in cmds {
                let ack = s.run_command(&entry, t)?;
                if ack.status != AckStatus::Rejected {
                    entry.error = (ack.status == AckStatus::Failed).then(|| ack.errors.join("; "));
                    kept.push(entry);
                }
                acks.push(ack);
            }
            Ok((acks, kept))
        })?;
        if !kept.is_empty() {
            let frame = self.frame_index(t);
            self.recorded.push(Batch { frame, cmds: kept.clone() });
            match self.batches.last_mut() {
                // Две пачки на одном кадре — одна: при воспроизведении они тоже пойдут подряд.
                Some(last) if last.frame == frame => last.cmds.extend(kept),
                _ => self.batches.push(Batch { frame, cmds: kept }),
            }
            self.next_batch = self.batches.len();
        }
        Ok(acks)
    }

    /// Снимок канала: что на каких слоях и с какими данными.
    pub fn state(&self) -> Option<&State> {
        self.channel.as_ref()
    }

    /// Пачки, принятые живыми командами с прошлого вызова: для записи журнала по ходу эфира.
    pub fn take_recorded(&mut self) -> Vec<Batch> {
        std::mem::take(&mut self.recorded)
    }

    /// Когда всё запущенное к этому моменту улеглось: кончились in и уходы. Кадр preview.
    pub fn quiet_time(&mut self) -> Result<f64> {
        let settled = self.settled_time()?;
        let leaving = self.releasing.iter().chain(&self.parked).filter_map(|r| self.motion.hide_time(*r)).fold(settled, f64::max);
        // Шаги перехода, которые ещё впереди, тоже должны отыграть.
        let leaving = self.plan.as_ref().map_or(leaving, |p| p.end().max(leaving));
        // Соседи ушедших переезжают после ухода.
        self.advance(self.snap(leaving).max(self.now))?;
        let moved = self.motion.layout_end().map_or(leaving, |e| e.max(leaving));
        Ok(self.snap(moved))
    }

    /// Команда слою в момент `t`.
    fn run_command(&mut self, entry: &Entry, t: f64) -> Result<Ack> {
        let ack = |status, errors: Vec<String>| Ack { id: entry.id.clone(), cmd: entry.cmd.label(), status, errors };
        let result = self.layer_command(&entry.cmd, t);
        // Слои, которые команда только потрогала (play на пустом), в снимке не нужны.
        if let Some(c) = &mut self.channel {
            c.layers.retain(|_, l| !l.is_empty());
        }
        let ack = match result {
            Ok(()) => ack(AckStatus::Ok, Vec::new()),
            Err(CmdError::Rejected(errors)) => return Ok(ack(AckStatus::Rejected, errors)),
            Err(CmdError::Failed(e)) => {
                // Исключение JS посреди команды: что успело измениться, применяется — как в эфире.
                let e = map_js(self.bundle.map.as_ref(), e);
                if e.downcast_ref::<keyfill_js::JsException>().is_none() {
                    return Err(e);
                }
                ack(AckStatus::Failed, vec![e.to_string()])
            }
        };
        self.motion.log.push(format!("{}@{}", entry.cmd.label(), keyfill_core::look::num(t as f32)));
        Ok(ack)
    }

    fn layer_command(&mut self, cmd: &Command, t: f64) -> Result<(), CmdError> {
        let reject = |m: String| Err(CmdError::Rejected(vec![m]));
        // Слоями состояний управляют состояния: прямые команды показа и данных на них — отказ.
        let direct = matches!(cmd, Command::Load { .. } | Command::Play { .. } | Command::Stop { .. } | Command::Update { .. } | Command::Clear { .. });
        if let Some(layer) = cmd.layer().filter(|_| direct)
            && let Some(name) = self.scene.channel.as_ref().and_then(|c| c.state_layer(layer))
        {
            return reject(format!(
                "{} {layer}: слой {name} ведут состояния канала; переключай state <имя>, данные — update без слоя",
                cmd.name()
            ));
        }
        match cmd {
            Command::State { name, instant } => self.enter_state(name, *instant, t),
            Command::Update { layer: None, props } => self.layer_js(serde_json::json!({ "op": "data", "props": props }), t),
            Command::Load { layer, component, props } => {
                let inst = self.mount(*layer, component, props, None, t)?;
                let slot = self.layer_mut(*layer);
                let old = slot.loaded.replace(inst);
                if let Some(old) = old {
                    self.unmount(&old, t)?;
                }
                Ok(())
            }
            Command::Play { layer, component: Some(component), props, instant } => {
                self.layer_command(&Command::Load { layer: *layer, component: component.clone(), props: props.clone() }, t)?;
                self.layer_command(&Command::Play { layer: *layer, component: None, props: Default::default(), instant: *instant }, t)
            }
            Command::Play { layer, component: None, props, instant } => {
                if !props.is_empty() {
                    return reject(format!("play {layer}: пропсы передаются вместе с компонентом: play {layer} <Компонент> {{…}}; для показанного — update"));
                }
                let slot = self.layer_mut(*layer);
                let Some(mut inst) = slot.loaded.take() else {
                    if let Some(cur) = &slot.current {
                        let msg = format!("play {layer}: слой уже показывает {}", cur.name);
                        self.event(t, "channel", "ignored", msg);
                        return Ok(());
                    }
                    return reject(format!("play {layer}: на слое ничего не загружено; play {layer} <Компонент> {{…}} или load"));
                };
                if let Some(old) = slot.current.take() {
                    self.leave(*layer, old, t)?;
                }
                self.event(t, &inst.name, "state", "playing".into());
                if *instant {
                    self.motion.play_settled(&self.scene, &[inst.root], t);
                } else {
                    self.motion.play(&self.scene, Some(&[inst.root]), t);
                }
                inst.state = InstState::Playing;
                self.layer_mut(*layer).current = Some(inst);
                Ok(())
            }
            Command::Next { layer } => {
                let root = self.shown(*layer, "next")?.root;
                self.motion.next(Some(&[root]), t);
                Ok(())
            }
            Command::Stop { layer } => {
                match self.layer_mut(*layer).current.take() {
                    Some(inst) => self.leave(*layer, inst, t)?,
                    None => self.event(t, "channel", "ignored", format!("stop {layer}: на слое ничего не показано")),
                }
                Ok(())
            }
            Command::Update { layer: Some(layer), props } => {
                let inst = self.target(*layer, "update")?.name.clone();
                self.layer_js(serde_json::json!({ "op": "update", "inst": inst, "props": props }), t)
            }
            Command::Invoke { layer, command, params } => {
                let inst = self.target(*layer, "invoke")?.name.clone();
                self.layer_js(serde_json::json!({ "op": "invoke", "inst": inst, "command": command, "params": params }), t)
            }
            Command::Clear { layer } => {
                let layers: Vec<u32> = match layer {
                    Some(l) => vec![*l],
                    None => self.channel.as_ref().map(|c| c.layers.keys().copied().collect()).unwrap_or_default(),
                };
                for l in layers {
                    let Some(slot) = self.channel.as_mut().and_then(|c| c.layers.remove(&l)) else { continue };
                    for inst in slot.all() {
                        self.parked.retain(|r| *r != inst.root);
                        self.unmount(inst, t)?;
                    }
                }
                if layer.is_none() {
                    self.pending.clear();
                    self.plan = None;
                    if let Some(c) = &mut self.channel {
                        c.state = None;
                    }
                }
                Ok(())
            }
        }
    }

    /// Переход в состояние канала — разница с текущим по слоям состояний: ушедшее уходит через
    /// `out`, пришедшее входит, тот же компонент на том же слое остаётся и переходит на пропсы
    /// нового состояния (через `transition`). Уходящий экземпляр, который снова нужен,
    /// разворачивается с того места, где он сейчас. Когда начинается каждый шаг, решает правило
    /// перехода (`transitions`): расписание сводится сразу, шаги не с начала ждут своего момента.
    fn enter_state(&mut self, name: &str, instant: bool, t: f64) -> Result<(), CmdError> {
        let def = self.scene.channel.clone().unwrap_or_default();
        if !def.states.contains_key(name) {
            let known: Vec<&str> = def.states.keys().map(String::as_str).collect();
            let known = if known.is_empty() { "ни одного: defineChannel({ states })".to_string() } else { known.join(", ") };
            return Err(CmdError::Rejected(vec![format!("state {name}: такого состояния нет; есть: {known}")]));
        }
        let from = self.channel.as_ref().and_then(|c| c.state.clone());
        if !instant && from.as_deref() == Some(name) {
            self.event(t, "channel", "ignored", format!("state {name}: канал уже в этом состоянии"));
            return Ok(());
        }
        // Прерывание: чего ещё не было, того не было — неначатые шаги прежнего перехода отменяются.
        self.cancel_pending(t)?;
        let steps = self.diff_steps(&def, name, instant, t)?;
        if instant {
            for s in &steps {
                self.run_step(s, name, true, t)?;
            }
            self.plan = None;
        } else {
            let durations = self.durations(&steps, t);
            let (rule, source) = def.rule(from.as_deref(), name);
            let names: BTreeMap<u32, String> = def.layers.iter().map(|(n, l)| (*l, n.clone())).collect();
            let acts: Vec<Act> = steps.iter().zip(&durations).map(|(s, d)| Act { layer: s.layer, kind: s.kind, dur: *d }).collect();
            let timed = transition::resolve(&rule, &acts, &names);
            self.check_overlap(&steps, &acts, &timed, &names, t);
            let plan = TransitionPlan {
                from,
                to: name.to_string(),
                t,
                rule: format!("{} ({source})", rule.label()),
                steps: steps
                    .iter()
                    .zip(acts.iter().zip(&timed))
                    .map(|(s, (a, tm))| PlanStep {
                        layer: s.layer,
                        name: names.get(&s.layer).cloned().unwrap_or_default(),
                        kind: s.kind,
                        inst: s.inst.clone(),
                        start: tm.start,
                        end: tm.start + a.dur,
                        why: tm.why.clone(),
                    })
                    .collect(),
            };
            for (s, tm) in steps.into_iter().zip(timed) {
                if tm.start <= 1e-9 {
                    self.run_step(&s, name, false, t)?;
                } else {
                    self.seq += 1;
                    self.pending.push(Pending { at: t + tm.start, seq: self.seq, step: s, state: name.to_string() });
                }
            }
            self.pending.sort_by(|a, b| (a.at, a.seq).partial_cmp(&(b.at, b.seq)).expect("время шага"));
            self.plan = Some(plan);
        }
        if let Some(c) = &mut self.channel {
            c.state = Some(name.to_string());
        }
        self.event(t, "channel", "state", name.to_string());
        Ok(())
    }

    /// Шаги перехода по слоям состояний в порядке номеров; учёт слоёв сразу ставится в цель:
    /// входящий смонтирован и ждёт своего шага, уходящий — среди уходящих, но ещё на экране.
    fn diff_steps(&mut self, def: &ChannelDef, name: &str, instant: bool, t: f64) -> Result<Vec<Step>, CmdError> {
        let target = &def.states[name];
        let mut layers: Vec<u32> = def.layers.values().copied().collect();
        layers.sort_unstable();
        let mut steps = Vec::new();
        for layer in layers {
            let want = target.get(&layer);
            let slot = self.layer_mut(layer);
            if let (Some(cur), Some(w)) = (&slot.current, want)
                && &cur.component == w
            {
                steps.push(Step { layer, kind: StepKind::Update, inst: cur.name.clone(), root: cur.root, revive: false });
                continue;
            }
            if let Some(cur) = slot.current.take() {
                steps.push(Step { layer, kind: StepKind::Out, inst: cur.name.clone(), root: cur.root, revive: false });
                slot.leaving.push(cur);
            }
            let Some(w) = want else { continue };
            let back = slot.leaving.iter().position(|i| &i.component == w && i.state == InstState::Leaving).filter(|_| !instant);
            match back {
                Some(i) => {
                    let inst = slot.leaving.remove(i);
                    steps.push(Step { layer, kind: StepKind::In, inst: inst.name.clone(), root: inst.root, revive: true });
                    // Ждёт разворота: снимать его по концу out уже нельзя, даже если шаг позже.
                    let root = inst.root;
                    slot.current = Some(inst);
                    self.parked.retain(|r| *r != root);
                }
                None => {
                    let inst = self.mount(layer, w, &Default::default(), Some(name), t)?;
                    steps.push(Step { layer, kind: StepKind::In, inst: inst.name.clone(), root: inst.root, revive: false });
                    self.layer_mut(layer).current = Some(inst);
                }
            }
        }
        Ok(steps)
    }

    /// Длительности шагов: те же команды движения на копии состояния. `update` — мгновенный шаг.
    fn durations(&self, steps: &[Step], t: f64) -> Vec<f64> {
        let mut probe = self.motion.clone();
        steps
            .iter()
            .map(|s| match (s.kind, s.revive) {
                (StepKind::Out, _) => {
                    probe.stop(&self.scene, Some(&[s.root]), t);
                    probe.hide_time(s.root).unwrap_or(t) - t
                }
                (StepKind::In, revive) => {
                    if revive {
                        probe.revive(&self.scene, &[s.root], t);
                    } else {
                        probe.play(&self.scene, Some(&[s.root]), t);
                    }
                    probe.in_end(&self.scene, s.root, t) - t
                }
                (StepKind::Update, _) => 0.0,
            })
            .collect()
    }

    /// Шаг перехода в момент `t`.
    fn run_step(&mut self, s: &Step, state: &str, instant: bool, t: f64) -> Result<(), CmdError> {
        match s.kind {
            StepKind::Update => self.layer_js(serde_json::json!({ "op": "bind", "inst": s.inst, "state": state, "layer": s.layer }), t),
            StepKind::Out => {
                let slot = self.layer_mut(s.layer);
                let Some(i) = slot.leaving.iter().position(|i| i.root == s.root) else { return Ok(()) };
                if instant {
                    let inst = slot.leaving.remove(i);
                    return self.unmount(&inst, t);
                }
                // Уходит без release: в JS экземпляр жив до конца out, его может развернуть
                // следующее состояние; снимает его sweep.
                slot.leaving[i].state = InstState::Leaving;
                self.event(t, &s.inst, "state", "stopping".into());
                self.motion.stop(&self.scene, Some(&[s.root]), t);
                self.parked.push(s.root);
                Ok(())
            }
            StepKind::In => {
                if let Some(cur) = self.layer_mut(s.layer).current.as_mut().filter(|c| c.root == s.root) {
                    cur.state = InstState::Playing;
                }
                self.event(t, &s.inst, "state", "playing".into());
                if s.revive {
                    self.parked.retain(|r| *r != s.root);
                    self.layer_js(serde_json::json!({ "op": "bind", "inst": s.inst, "state": state, "layer": s.layer }), t)?;
                    self.motion.revive(&self.scene, &[s.root], t);
                } else if instant {
                    self.motion.play_settled(&self.scene, &[s.root], t);
                } else {
                    self.motion.play(&self.scene, Some(&[s.root]), t);
                }
                Ok(())
            }
        }
    }

    /// Отменяет неначатые шаги перехода: невошедший снимается (его не видели), неушедший
    /// возвращается на слой, неразвёрнутый остаётся уходить, необновлённый — на прежней привязке.
    fn cancel_pending(&mut self, t: f64) -> Result<(), CmdError> {
        let pending = std::mem::take(&mut self.pending);
        for p in pending.iter().filter(|p| p.step.kind == StepKind::In) {
            let s = &p.step;
            let Some(mut inst) = self.layer_mut(s.layer).current.take_if(|c| c.root == s.root) else { continue };
            if s.revive {
                inst.state = InstState::Leaving;
                self.layer_mut(s.layer).leaving.push(inst);
                self.parked.push(s.root);
            } else {
                self.unmount(&inst, t)?;
            }
        }
        for p in pending.iter().filter(|p| p.step.kind == StepKind::Out) {
            let slot = self.layer_mut(p.step.layer);
            if let Some(i) = slot.leaving.iter().position(|i| i.root == p.step.root) {
                let inst = slot.leaving.remove(i);
                slot.current = Some(inst);
            }
        }
        Ok(())
    }

    /// Следующий отложенный шаг перехода, в свой момент.
    fn run_pending(&mut self) -> Result<()> {
        let p = self.pending.remove(0);
        self.motion.advance_to(p.at);
        self.sweep(p.at)?;
        self.relayout(p.at, |s| {
            s.run_step(&p.step, &p.state, false, p.at).map_err(|e| match e {
                CmdError::Failed(e) => e,
                CmdError::Rejected(errors) => anyhow::anyhow!(errors.join("; ")),
            })
        })
    }

    /// Конфликт расписания: входящий въезжает в область, которую ещё занимает уходящий на
    /// другом слое (warn `transition.overlap`).
    fn check_overlap(&mut self, steps: &[Step], acts: &[Act], timed: &[transition::Timed], names: &BTreeMap<u32, String>, t: f64) {
        let ins: Vec<usize> = (0..steps.len()).filter(|i| steps[*i].kind == StepKind::In).collect();
        let outs: Vec<usize> = (0..steps.len()).filter(|i| steps[*i].kind == StepKind::Out).collect();
        if ins.is_empty() || outs.is_empty() {
            return;
        }
        let reveal: Vec<u32> = ins.iter().map(|i| steps[*i].root).collect();
        let Ok(geo) = self.geometry_with(t, &reveal) else { return };
        let rect = |root: u32| geo.get(&root).map(|g| Rect { x: g.rel[0], y: g.rel[1], w: g.rel[2], h: g.rel[3] });
        for &i in &ins {
            for &o in &outs {
                let (a, b) = (&steps[i], &steps[o]);
                let out_end = timed[o].start + acts[o].dur;
                if a.layer == b.layer || timed[i].start >= out_end - 1e-9 {
                    continue;
                }
                let (Some(ra), Some(rb)) = (rect(a.root), rect(b.root)) else { continue };
                if !ra.intersects(&rb) {
                    continue;
                }
                let (la, lb) = (names.get(&a.layer).cloned().unwrap_or_default(), names.get(&b.layer).cloned().unwrap_or_default());
                let msg = format!(
                    "{la} ({}) входит в {} с, пока {lb} ({}) уходит до {} с: их области пересекаются",
                    a.inst,
                    keyfill_core::look::num(timed[i].start as f32),
                    b.inst,
                    keyfill_core::look::num(out_end as f32)
                );
                let hint = format!("начни вход после ухода: enter('{la}', {{ after: '{lb}:out' }}) или sequence()");
                self.motion.diagnostics.push(Level::Warn, "transition.overlap", &a.inst, None, msg, Some(&hint));
            }
        }
    }

    fn layer_mut(&mut self, layer: u32) -> &mut Layer {
        self.channel.as_mut().expect("канал").layers.entry(layer).or_default()
    }

    /// Показанный экземпляр слоя.
    fn shown(&self, layer: u32, what: &str) -> Result<&Inst, CmdError> {
        let slot = self.channel.as_ref().and_then(|c| c.layers.get(&layer));
        slot.and_then(|s| s.current.as_ref())
            .ok_or_else(|| CmdError::Rejected(vec![format!("{what} {layer}: на слое ничего не показано")]))
    }

    /// Экземпляр для данных и команд: показанный, а если его нет — загруженный.
    fn target(&self, layer: u32, what: &str) -> Result<&Inst, CmdError> {
        let slot = self.channel.as_ref().and_then(|c| c.layers.get(&layer));
        slot.and_then(|s| s.current.as_ref().or(s.loaded.as_ref()))
            .ok_or_else(|| CmdError::Rejected(vec![format!("{what} {layer}: на слое никого нет")]))
    }

    /// Экземпляр уходит через `out`; удалится, когда уход кончится.
    /// Экземпляр отпускается: JS снимает его эффекты, ядро ведёт `out` и удалит узлы после ухода.
    fn leave(&mut self, layer: u32, mut inst: Inst, t: f64) -> Result<(), CmdError> {
        self.event(t, &inst.name, "state", "stopping".into());
        self.layer_js(serde_json::json!({ "op": "release", "inst": inst.name }), t)?;
        inst.state = InstState::Leaving;
        self.layer_mut(layer).leaving.push(inst);
        Ok(())
    }

    /// Удаляет отпущенные узлы, уход которых кончился до `t`; у канала — и уходящие экземпляры
    /// со слоёв, с событием `removed` в момент конца ухода.
    fn sweep(&mut self, t: f64) -> Result<()> {
        let mut gone = BTreeMap::new();
        let motion = &self.motion;
        let due = |root: &u32, gone: &mut BTreeMap<u32, f64>| match motion.hide_time(*root) {
            Some(at) if at <= t + 1e-9 => {
                gone.insert(*root, at);
                false
            }
            _ => true,
        };
        self.releasing.retain(|r| due(r, &mut gone));
        let mut parked = BTreeMap::new();
        self.parked.retain(|r| due(r, &mut parked));
        for &root in gone.keys() {
            if self.scene.nodes.contains_key(&root) {
                self.scene.remove(root)?;
            }
        }
        // Экземпляры состояний живут в JS до конца ухода: снимает их JS, со своими эффектами.
        for &root in parked.keys() {
            let inst = self.channel.as_ref().and_then(|c| c.layers.values().flat_map(|l| l.leaving.iter()).find(|i| i.root == root)).map(|i| i.name.clone());
            if let Some(inst) = inst {
                self.layer_js_replies(serde_json::json!({ "op": "unmount", "inst": inst }), t).map_err(|e| match e {
                    CmdError::Failed(e) => e,
                    CmdError::Rejected(errors) => anyhow::anyhow!(errors.join("; ")),
                })?;
            }
        }
        gone.extend(parked);
        let removed = std::mem::take(&mut self.scene.removed);
        if !removed.is_empty() {
            self.motion.forget(&removed);
        }
        let Some(state) = &mut self.channel else { return Ok(()) };
        let mut events = Vec::new();
        for slot in state.layers.values_mut() {
            slot.leaving.retain(|inst| {
                let alive = !gone.contains_key(&inst.root);
                if !alive {
                    events.push((gone[&inst.root], inst.name.clone()));
                }
                alive
            });
        }
        state.layers.retain(|_, l| !l.is_empty());
        for (at, name) in events {
            self.event(at, &name, "state", "removed".into());
        }
        Ok(())
    }

    /// Монтирует экземпляр компонента на слой: в дереве он встаёт над нижними слоями и под
    /// верхними, на своём слое — поверх прежних.
    fn mount(&mut self, layer: u32, component: &str, props: &serde_json::Map<String, serde_json::Value>, state: Option<&str>, t: f64) -> Result<Inst, CmdError> {
        let above: Vec<u32> = self
            .channel
            .as_ref()
            .map(|c| c.layers.range(layer + 1..).flat_map(|(_, l)| l.all().map(|i| i.root)).collect())
            .unwrap_or_default();
        let stage = self.scene.stage.as_ref().map(|s| s.id).unwrap_or(0);
        let before = self.scene.nodes.get(&stage).and_then(|s| s.children.iter().find(|c| above.contains(c)).copied());
        let mut arg = serde_json::json!({ "op": "mount", "component": component, "props": props, "before": before });
        if let Some(state) = state {
            arg["state"] = state.into();
            arg["layer"] = layer.into();
        }
        let replies = self.layer_js_replies(arg, t)?;
        let mut inst = None;
        for r in replies {
            match r {
                LayerReply::Mounted { inst: name, root } => {
                    inst = Some(Inst { name, component: component.to_string(), root, state: InstState::Loaded, props: serde_json::Value::Null })
                }
                LayerReply::Values { props, .. } => {
                    if let Some(i) = &mut inst {
                        i.props = props;
                    }
                }
                LayerReply::Rejected { .. } | LayerReply::Data { .. } => {}
            }
        }
        let inst = inst.ok_or_else(|| CmdError::Failed(anyhow::anyhow!("канал: JS не сообщил о монтировании {component}")))?;
        self.motion.hold_loaded(inst.root);
        self.event(t, &inst.name, "state", format!("loaded {layer}"));
        Ok(inst)
    }

    fn unmount(&mut self, inst: &Inst, t: f64) -> Result<(), CmdError> {
        self.layer_js_replies(serde_json::json!({ "op": "unmount", "inst": inst.name }), t)?;
        self.event(t, &inst.name, "state", "removed".into());
        Ok(())
    }

    fn layer_js(&mut self, arg: serde_json::Value, t: f64) -> Result<(), CmdError> {
        self.layer_js_replies(arg, t).map(drop)
    }

    /// Ответы `values`: текущие пропсы экземпляров на слоях.
    fn take_values(&mut self, replies: &[LayerReply]) {
        let Some(c) = &mut self.channel else { return };
        for r in replies {
            if let LayerReply::Data { props } = r {
                c.data = props.clone();
            }
            let LayerReply::Values { inst, props } = r else { continue };
            for l in c.layers.values_mut() {
                for i in l.current.iter_mut().chain(l.loaded.iter_mut()) {
                    if &i.name == inst {
                        i.props = props.clone();
                    }
                }
            }
        }
    }

    /// Вызов `__keyfill_channel`: пакеты применяются (и при исключении — то, что JS успел
    /// отправить), удалённые узлы забываются, изменения анимируются, команды движения из
    /// `ctx` исполняются в тот же момент.
    fn layer_js_replies(&mut self, arg: serde_json::Value, t: f64) -> Result<Vec<LayerReply>, CmdError> {
        let result = self.js.call("__keyfill_channel", &arg.to_string());
        let out = match &result {
            Ok(_) => None,
            Err(_) => Some(self.js.take_output()),
        };
        let out = match result {
            Ok(out) => out,
            Err(e) => {
                self.absorb(out.unwrap_or_default(), t, None).map_err(CmdError::Failed)?;
                let replies = std::mem::take(&mut self.scene.replies);
                self.take_values(&replies);
                return Err(CmdError::Failed(e));
            }
        };
        self.absorb(out, t, None).map_err(CmdError::Failed)?;
        let replies = std::mem::take(&mut self.scene.replies);
        self.take_values(&replies);
        if let Some(LayerReply::Rejected { errors }) = replies.iter().find(|r| matches!(r, LayerReply::Rejected { .. })) {
            return Err(CmdError::Rejected(errors.clone()));
        }
        Ok(replies)
    }

    /// Применяет то, что отправил JS: пакеты, удаления, движение.
    /// Применяет то, что отправил JS: пакеты, удаления, движение, уходы. `label` — команда
    /// сценария для сообщений об ошибках движения.
    fn absorb(&mut self, out: keyfill_js::RunOutput, t: f64, label: Option<&str>) -> Result<()> {
        self.logs.extend(out.logs);
        for batch in &out.batches {
            self.scene.apply_batch(batch)?;
        }
        let removed = std::mem::take(&mut self.scene.removed);
        if !removed.is_empty() {
            self.motion.forget(&removed);
        }
        self.motion.observe(&self.scene, t);
        // Уход — после observe: узел, созданный и отпущенный одной командой, уже известен движению.
        let released = std::mem::take(&mut self.scene.released);
        if !released.is_empty() {
            self.motion.release(&self.scene, &released, t);
            self.releasing.extend(released);
        }
        for m in std::mem::take(&mut self.scene.motion_cmds) {
            if let Err(e) = self.motion.instance_cmd(&self.scene, &m.inst, &m.action, t) {
                let msg = label.map_or(e.clone(), |l| format!("{l}: {e}"));
                self.motion.diagnostics.push(Level::Error, "command.motion", &m.inst, None, msg, None);
            }
        }
        Ok(())
    }

    fn event(&mut self, t: f64, source: &str, kind: &'static str, name: String) {
        self.motion.push_event(MotionEvent { t, source: source.into(), kind, name });
    }

    /// Вызов JS на команде сценария: пакеты применяются, изменения анимируются, а команды
    /// движения из `ctx` исполняются в тот же момент.
    fn call_js(&mut self, name: &str, arg: &str, cue: &Cue) -> Result<()> {
        let out = self.js.call(name, arg).map_err(|e| map_js(self.bundle.map.as_ref(), e)).with_context(|| format!("команда {}", cue.label()))?;
        self.absorb(out, cue.t, Some(&cue.label()))
    }

    /// Момент, когда воспроизведение впервые проходит маркер таймлайна.
    pub fn marker_time(&mut self, name: &str) -> Result<f64> {
        const LIMIT: f64 = 600.0;
        loop {
            if let Some(e) = self.motion.events_log.iter().find(|e| e.kind == "marker" && e.name == name) {
                return Ok(e.t);
            }
            let cue = self.next_command_time();
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

    /// Кадр и время его стадий — для `keyfill perf`.
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
        // Расписание перехода — пока он идёт.
        let transition = self.plan.clone().filter(|p| self.labels && t + 1e-9 >= p.t && t <= p.end() + 1e-9);
        anim.clock = Some(Clock { t, frame: self.frame_index(t), scene: self.motion.scene_phase(), log, events, transition });
        let motion = start.elapsed() - script;
        let frame = evaluate_at(&self.scene, &mut self.engine, eval, &anim, &mut self.hints)?;
        let layout = start.elapsed() - script - motion;
        Ok((frame, StageTimes { script, motion, layout }))
    }

    /// Кадр «в эфире»: момент, когда закончились все `in` первого play (на границе кадра).
    /// У канала — когда успокоилось всё, что запустила последняя пачка журнала.
    pub fn settled_time(&mut self) -> Result<f64> {
        if self.channel.is_some() {
            let last = self.batches.last().map_or(0.0, |b| self.batch_time(b));
            self.advance(last.max(self.now))?;
            return Ok(self.snap(self.motion.settled_at.unwrap_or(0.0).max(last)));
        }
        if self.settled.is_none() {
            let Some(play) = self.cues.iter().find(|c| c.cmd == CueCmd::Play).map(|c| c.t) else { return Ok(0.0) };
            self.advance(play)?;
        }
        Ok(self.snap(self.settled.unwrap_or(0.0)))
    }

    /// Конец сценария: последняя команда и всё, что она запустила.
    pub fn end_time(&mut self) -> Result<f64> {
        let last_batch = self.batches.last().map_or(0.0, |b| self.batch_time(b));
        let last = self.cues.last().map_or(0.0, |c| c.t).max(last_batch);
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

/// Шаг перехода на слое состояния.
#[derive(Debug, Clone)]
struct Step {
    layer: u32,
    kind: StepKind,
    inst: String,
    root: u32,
    /// Вход — разворот уходящего, а не новый экземпляр.
    revive: bool,
}

/// Шаг, который ждёт своего момента.
#[derive(Debug, Clone)]
struct Pending {
    at: f64,
    seq: u64,
    step: Step,
    /// Состояние, ради которого шаг: по нему привязка пропсов.
    state: String,
}

/// Почему команда слою не прошла.
enum CmdError {
    /// Не исполнялась: состояние не менялось.
    Rejected(Vec<String>),
    /// Упала посреди исполнения (исключение JS) или сбой keyfill.
    Failed(anyhow::Error),
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
