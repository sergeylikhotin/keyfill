//! Таймлайны компонентов (`*.motion.ts`): ключи во времени таймлайна τ, маркеры, цикл,
//! остановки и события.
//!
//! Воспроизведение — кусочно-линейное отображение времени сцены `t` в τ: вперёд, назад
//! (pingpong) или стоп (hold). Поэтому значение в любой момент вычисляется в замкнутой форме,
//! без проигрывания кадров. Прыжки (stop → `out`, сегмент команды) продолжают каналы с текущих
//! значений; каналы, которым после прыжка нечего делать, доигрывают своё движение во времени сцены.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::motion::{Channel, Compiler, EASE_IN_OUT, Easing, Seg, Track, V};

const EPS: f64 = 1e-9;

#[derive(Debug, Clone)]
pub struct Key {
    pub t: f64,
    pub part: String,
    pub channel: Channel,
    pub from: Option<V>,
    pub to: V,
    pub easing: Easing,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LoopDef {
    pub from: String,
    pub to: String,
    pub a: f64,
    pub b: f64,
    pub pingpong: bool,
    /// Сколько раз сыграть отрезок; `None` — до next или stop.
    pub count: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct TimelineDef {
    pub component: String,
    pub keys: Vec<Key>,
    pub markers: BTreeMap<String, f64>,
    pub repeat: Option<LoopDef>,
    pub holds: Vec<String>,
    pub emits: Vec<(f64, String)>,
    /// Последний момент, когда что-то происходит.
    pub end: f64,
}

impl TimelineDef {
    /// Маркеры по возрастанию времени таймлайна.
    pub fn markers_by_time(&self) -> Vec<(&str, f64)> {
        let mut out: Vec<(&str, f64)> = self.markers.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        out.sort_by(|a, b| a.1.total_cmp(&b.1));
        out
    }

    fn next_marker_after(&self, x: f64) -> Option<f64> {
        self.markers.values().copied().filter(|m| *m > x + EPS).reduce(f64::min)
    }

    /// Первая остановка при play с нуля: hold, начало цикла, `out` или конец — «в эфире».
    pub fn settle_tau(&self) -> f64 {
        let mut best = self.end;
        for h in &self.holds {
            best = best.min(self.markers[h]);
        }
        if let Some(l) = &self.repeat {
            best = best.min(l.a);
        }
        if let Some(o) = self.markers.get("out") {
            best = best.min(*o);
        }
        best
    }
}

/// Разбирает таймлайн из JS. Ошибки собираются все сразу: агенту полезнее список, чем первая.
pub(crate) fn parse(v: &Value, c: &mut Compiler) -> Result<TimelineDef, Vec<String>> {
    let mut errors = Vec::new();
    let component = v.get("timeline").and_then(Value::as_str).unwrap_or("?").to_string();
    let default_easing = Easing::Tween { duration: 0.3, curve: EASE_IN_OUT };

    let mut keys = Vec::new();
    for (i, k) in v.get("keys").and_then(Value::as_array).into_iter().flatten().enumerate() {
        let t = k.get("t").and_then(Value::as_f64).filter(|t| *t >= 0.0);
        let part = k.get("part").and_then(Value::as_str);
        let (Some(t), Some(part)) = (t, part) else {
            errors.push(format!("keys[{i}]: ожидается at(t ≥ 0, part, targets)"));
            continue;
        };
        let easing = match k.get("easing").filter(|e| !e.is_null()) {
            Some(e) => match Easing::parse(e) {
                Ok(e) => e,
                Err(e) => {
                    errors.push(format!("keys[{i}]: {e}"));
                    continue;
                }
            },
            None => default_easing,
        };
        for (name, raw) in k.get("targets").and_then(Value::as_object).into_iter().flatten() {
            let Some(channel) = Channel::parse(name) else {
                // Канал, которого ядро не умеет анимировать (blur), пропускается с предупреждением,
                // как в presence: остальной таймлайн работает.
                let what = if name == "blur" { "blur пока не анимируется" } else { "неизвестный канал" };
                c.notes.push((crate::diag::Level::Warn, "motion.unsupported", format!("keys[{i}].{name}: {what}, пропущен")));
                continue;
            };
            let pair = raw.as_array().filter(|p| p.len() == 2);
            let parsed = match pair {
                Some(p) => c.value(channel, &p[0]).and_then(|a| Ok((Some(a), c.value(channel, &p[1])?))),
                None => c.value(channel, raw).map(|b| (None, b)),
            };
            match parsed {
                Ok((from, to)) => keys.push(Key { t, part: part.to_string(), channel, from, to, easing }),
                Err(e) => errors.push(format!("keys[{i}].{name}: {e}")),
            }
        }
    }

    let mut markers = BTreeMap::new();
    for (name, m) in v.get("markers").and_then(Value::as_object).into_iter().flatten() {
        match m.as_f64().filter(|m| *m >= 0.0) {
            Some(m) => {
                markers.insert(name.clone(), m);
            }
            None => errors.push(format!("markers.{name}: ожидается время ≥ 0")),
        }
    }
    let marker = |name: &str, what: &str, errors: &mut Vec<String>| -> Option<f64> {
        let m = markers.get(name).copied();
        if m.is_none() {
            let known = markers.keys().cloned().collect::<Vec<_>>().join(", ");
            errors.push(format!("{what}: маркера {name:?} нет; маркеры: {known}"));
        }
        m
    };

    let mut repeat = None;
    if let Some(l) = v.get("loop").filter(|l| l.is_object()) {
        let from = l.get("from").and_then(Value::as_str).unwrap_or_default().to_string();
        let to = l.get("to").and_then(Value::as_str).unwrap_or_default().to_string();
        let a = marker(&from, "loop.from", &mut errors);
        let b = marker(&to, "loop.to", &mut errors);
        if let (Some(a), Some(b)) = (a, b) {
            if b <= a {
                errors.push(format!("loop: {to} ({b}) должен быть позже {from} ({a})"));
            } else {
                let pingpong = l.get("mode").and_then(Value::as_str) == Some("pingpong");
                let count = match l.get("count") {
                    None | Some(Value::Null) => None,
                    Some(c) => match c.as_u64().filter(|n| *n >= 1) {
                        Some(n) => Some(n as u32),
                        None => {
                            errors.push(format!("loop.count: ожидается целое ≥ 1, получено {c}"));
                            None
                        }
                    },
                };
                repeat = Some(LoopDef { from, to, a, b, pingpong, count });
            }
        }
    } else if let (Some(&a), Some(&b)) = (markers.get("loop"), markers.get("out"))
        && b > a {
            repeat = Some(LoopDef { from: "loop".into(), to: "out".into(), a, b, pingpong: false, count: None });
        }

    let mut holds = Vec::new();
    for h in v.get("holds").and_then(Value::as_array).into_iter().flatten() {
        let name = h.as_str().unwrap_or_default();
        if marker(name, "holds", &mut errors).is_some() {
            holds.push(name.to_string());
        }
    }

    let mut emits = Vec::new();
    for e in v.get("events").and_then(Value::as_array).into_iter().flatten() {
        match (e.get("t").and_then(Value::as_f64), e.get("name").and_then(Value::as_str)) {
            (Some(t), Some(name)) if t >= 0.0 => emits.push((t, name.to_string())),
            _ => errors.push(format!("events: ожидается emit(t ≥ 0, name), получено {e}")),
        }
    }

    let end = keys
        .iter()
        .map(|k| k.t + k.easing.duration(0.0))
        .chain(markers.values().copied())
        .chain(emits.iter().map(|e| e.0))
        .fold(0.0, f64::max);
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(TimelineDef { component, keys, markers, repeat, holds, emits, end })
}

// ─── Воспроизведение экземпляра ─────────────────────────────────────────────

pub type KeyId = (u32, Channel);

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Loaded,
    Main,
    Segment { end: f64 },
    Out { end: f64 },
    Finished,
    Removed,
}

#[derive(Debug, Clone, Copy)]
struct Head {
    t0: f64,
    tau0: f64,
    /// 1 — вперёд, -1 — назад (pingpong), 0 — стоит.
    dir: f64,
}

impl Head {
    fn tau(&self, t: f64) -> f64 {
        self.tau0 + self.dir * (t - self.t0)
    }
}

#[derive(Debug, Clone)]
struct Saved {
    tau: f64,
    dir: f64,
    loop_on: bool,
    loop_left: Option<u32>,
    mode: Mode,
    state: RunState,
}

/// Состояние воспроизведения экземпляра: событие `state:<имя>` и пометка в дампе.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RunState {
    Loaded,
    In,
    Hold,
    Loop,
    Segment,
    Out,
    Finished,
    Removed,
}

impl RunState {
    pub fn label(self) -> &'static str {
        match self {
            RunState::Loaded => "loaded",
            RunState::In => "in",
            RunState::Hold => "hold",
            RunState::Loop => "loop",
            RunState::Segment => "segment",
            RunState::Out => "out",
            RunState::Finished => "finished",
            RunState::Removed => "removed",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stop {
    Hold(String),
    LoopEnd,
    LoopStart,
    End,
    SegmentEnd,
    OutEnd,
}

/// Событие воспроизведения: смена состояния, маркер, своё событие, конец.
#[derive(Debug, Clone, PartialEq)]
pub struct Ev {
    pub t: f64,
    pub kind: &'static str,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct Run {
    pub inst: String,
    pub root: u32,
    pub def: TimelineDef,
    base: BTreeMap<KeyId, Track>,
    active: BTreeMap<KeyId, Track>,
    detached: BTreeMap<KeyId, Track>,
    head: Head,
    mode: Mode,
    loop_on: bool,
    loop_left: Option<u32>,
    saved: Option<Saved>,
    last_tau: f64,
    pub state: RunState,
    pub events: Vec<Ev>,
    pub generation: u64,
}

impl Run {
    /// `start` — начальное значение канала, если у первого ключа нет явного `from`.
    pub fn new(inst: String, root: u32, def: TimelineDef, nodes: &BTreeMap<String, u32>, start: impl Fn(u32, Channel) -> V) -> Run {
        let mut grouped: BTreeMap<KeyId, Vec<&Key>> = BTreeMap::new();
        for k in &def.keys {
            if let Some(&id) = nodes.get(&k.part) {
                grouped.entry((id, k.channel)).or_default().push(k);
            }
        }
        let mut base = BTreeMap::new();
        for (key, mut ks) in grouped {
            ks.sort_by(|a, b| a.t.total_cmp(&b.t));
            let mut prev = start(key.0, key.1);
            let segs = ks
                .iter()
                .map(|k| {
                    let from = k.from.unwrap_or(prev);
                    prev = k.to;
                    Seg::new(k.t, from, k.to, k.easing, 0.0, false)
                })
                .collect();
            base.insert(key, Track { segs, source: "timeline" });
        }
        Run {
            inst,
            root,
            def,
            active: base.clone(),
            base,
            detached: BTreeMap::new(),
            head: Head { t0: 0.0, tau0: 0.0, dir: 0.0 },
            mode: Mode::Loaded,
            loop_on: false,
            loop_left: None,
            saved: None,
            last_tau: 0.0,
            state: RunState::Loaded,
            events: Vec::new(),
            generation: 0,
        }
    }

    pub fn keys(&self) -> impl Iterator<Item = &KeyId> {
        self.base.keys()
    }

    pub fn tau(&self, t: f64) -> f64 {
        self.head.tau(t)
    }

    pub fn removed(&self) -> bool {
        self.mode == Mode::Removed
    }

    pub fn is_looping(&self) -> bool {
        self.state == RunState::Loop
    }

    pub fn value(&self, key: KeyId, t: f64) -> Option<V> {
        if let Some(d) = self.detached.get(&key) {
            return Some(d.value(t));
        }
        self.active.get(&key).map(|tr| tr.value(self.head.tau(t)))
    }

    /// Дорожка и момент для inspect: изинг и прогресс считаются в её собственном времени.
    pub fn track(&self, key: KeyId, t: f64) -> Option<(&Track, f64)> {
        if let Some(d) = self.detached.get(&key) {
            return Some((d, t));
        }
        self.active.get(&key).map(|tr| (tr, self.head.tau(t)))
    }

    fn set_state(&mut self, t: f64, state: RunState) {
        if self.state != state {
            self.state = state;
            self.events.push(Ev { t, kind: "state", name: state.label().to_string() });
        }
    }

    fn event(&mut self, t: f64, kind: &'static str, name: &str) {
        self.events.push(Ev { t, kind, name: name.to_string() });
    }

    fn marker_at(&self, tau: f64) -> Option<String> {
        self.def.markers.iter().find(|(_, m)| (**m - tau).abs() < EPS).map(|(n, _)| n.clone())
    }

    /// События пересечения маркеров и `emit` между прошлой обработкой и моментом `t`.
    pub fn flush(&mut self, t: f64) {
        self.flush_to(t, false);
    }

    /// `exclusive` — маркер ровно в `t` не считается пройденным: на нём кончается сегмент,
    /// `out` или виток цикла, и воспроизведение туда не заходит.
    fn flush_to(&mut self, t: f64, exclusive: bool) {
        if self.head.dir == 0.0 || !matches!(self.mode, Mode::Main | Mode::Segment { .. } | Mode::Out { .. }) {
            self.last_tau = self.head.tau(t);
            return;
        }
        let now = self.head.tau(t);
        let last = self.last_tau;
        if (now - last).abs() < EPS {
            return;
        }
        let fwd = now > last;
        let end_ok = |x: f64| !exclusive || (x - now).abs() > EPS;
        let inside = |x: f64| end_ok(x) && if fwd { x > last + EPS && x <= now + EPS } else { x < last - EPS && x >= now - EPS };
        let mut hits: Vec<(f64, &'static str, String)> = Vec::new();
        for (name, &m) in &self.def.markers {
            if inside(m) {
                hits.push((m, "marker", name.clone()));
            }
        }
        if fwd {
            for (at, name) in &self.def.emits {
                if inside(*at) {
                    hits.push((*at, "emit", name.clone()));
                }
            }
        }
        hits.sort_by(|a, b| if fwd { a.0.total_cmp(&b.0) } else { b.0.total_cmp(&a.0) });
        let loop_from = self.def.repeat.as_ref().map(|l| l.from.clone());
        for (x, kind, name) in hits {
            let tc = self.head.t0 + (x - self.head.tau0) / self.head.dir;
            self.event(tc, kind, &name);
            if kind == "marker" && fwd && self.loop_on && self.mode == Mode::Main && loop_from.as_deref() == Some(name.as_str()) {
                self.set_state(tc, RunState::Loop);
            }
        }
        self.last_tau = now;
    }

    /// Следующая остановка в текущем направлении: (момент сцены, что там).
    pub fn next_stop(&self) -> Option<(f64, Stop)> {
        let h = self.head;
        if h.dir == 0.0 {
            return None;
        }
        let tau = h.tau0;
        let mut best: Option<(f64, Stop)> = None;
        let mut consider = |x: f64, s: Stop| {
            let ahead = if h.dir > 0.0 { x > tau + EPS } else { x < tau - EPS };
            if ahead && best.as_ref().is_none_or(|(b, _)| (x - tau).abs() < (b - tau).abs()) {
                best = Some((x, s));
            }
        };
        match self.mode {
            Mode::Main if h.dir > 0.0 => {
                for name in &self.def.holds {
                    consider(self.def.markers[name], Stop::Hold(name.clone()));
                }
                if self.loop_on
                    && let Some(l) = &self.def.repeat {
                        consider(l.b, Stop::LoopEnd);
                    }
                if let Some(&out) = self.def.markers.get("out") {
                    consider(out, Stop::Hold("out".into()));
                }
                consider(self.def.end, Stop::End);
            }
            Mode::Main => {
                if let Some(l) = &self.def.repeat {
                    consider(l.a, Stop::LoopStart);
                }
            }
            Mode::Segment { end } => consider(end, Stop::SegmentEnd),
            Mode::Out { end } => consider(end, Stop::OutEnd),
            _ => {}
        }
        // Моменты остановок на сетке 1e-9 с: витки цикла копят погрешность сложения, и без
        // сетки граница витка попадала то до кадра, то после него.
        best.map(|(x, s)| (((h.t0 + (x - tau) / h.dir) * 1e9).round() / 1e9, s))
    }

    /// Обрабатывает остановку, которую вернул `next_stop`, в её момент.
    pub fn on_stop(&mut self, t: f64, stop: Stop) {
        // Маркер, на котором воспроизведение останавливается или разворачивается, не считается
        // пройденным: `out` проходится только по stop.
        let exclusive = matches!(stop, Stop::LoopEnd | Stop::SegmentEnd | Stop::OutEnd) || stop == Stop::Hold("out".into());
        self.flush_to(t, exclusive);
        let tau = self.head.tau(t);
        match stop {
            Stop::Hold(_) => {
                self.head = Head { t0: t, tau0: tau, dir: 0.0 };
                self.set_state(t, RunState::Hold);
            }
            Stop::LoopEnd => {
                let l = self.def.repeat.clone().expect("loop");
                if self.loop_left == Some(1) {
                    // Повторы кончились: дальше вперёд, а если здесь hold или out — стоим.
                    self.loop_on = false;
                    let stays = self.def.holds.iter().any(|h| (self.def.markers[h] - tau).abs() < EPS)
                        || self.def.markers.get("out").is_some_and(|o| (o - tau).abs() < EPS);
                    self.head = Head { t0: t, tau0: tau, dir: if stays { 0.0 } else { 1.0 } };
                    self.set_state(t, if stays { RunState::Hold } else { RunState::In });
                    return;
                }
                if let Some(n) = &mut self.loop_left {
                    *n -= 1;
                }
                // Каналы снова идут по исходным ключам: после сегмента команды они были перестроены
                // от текущих значений только вперёд, и назад (pingpong) застывали бы.
                self.active = self.base.clone();
                self.detached.clear();
                if l.pingpong {
                    self.head = Head { t0: t, tau0: l.b, dir: -1.0 };
                } else {
                    // Повтор: цикл задуман бесшовным.
                    self.head = Head { t0: t, tau0: l.a, dir: 1.0 };
                    self.last_tau = l.a;
                    self.event(t, "marker", &l.from);
                }
            }
            Stop::LoopStart => {
                self.active = self.base.clone();
                self.detached.clear();
                self.head = Head { t0: t, tau0: tau, dir: 1.0 };
            }
            Stop::End => {
                self.head = Head { t0: t, tau0: tau, dir: 0.0 };
                self.mode = Mode::Finished;
                self.set_state(t, RunState::Finished);
            }
            Stop::SegmentEnd => {
                let saved = self.saved.take().expect("saved");
                self.jump(t, saved.tau);
                self.head = Head { t0: t, tau0: saved.tau, dir: saved.dir };
                self.loop_on = saved.loop_on;
                self.loop_left = saved.loop_left;
                self.mode = saved.mode;
                self.set_state(t, saved.state);
            }
            Stop::OutEnd => {
                self.head = Head { t0: t, tau0: tau, dir: 0.0 };
                self.mode = Mode::Removed;
                self.set_state(t, RunState::Removed);
            }
        }
    }

    /// Прыжок в `to`: каналы с ключами после прыжка продолжают с текущих значений,
    /// остальные доигрывают текущее движение во времени сцены.
    fn jump(&mut self, t: f64, to: f64) {
        let tau_old = self.head.tau(t);
        let mut active = BTreeMap::new();
        let mut detached = BTreeMap::new();
        for (key, track) in &self.base {
            let cur = self.value(*key, t).unwrap_or(track.segs[0].from);
            let mut segs: Vec<Seg> = Vec::new();
            for s in &track.segs {
                let end = s.start + s.dur;
                if s.start < to - EPS {
                    if end <= to + EPS {
                        continue;
                    }
                    segs.push(Seg::new(to, cur, s.to, remaining(s.easing, end - to), 0.0, false));
                } else {
                    let from = if segs.is_empty() { cur } else { s.from };
                    segs.push(Seg::new(s.start, from, s.to, s.easing, 0.0, false));
                }
            }
            if segs.is_empty() {
                let cont = match self.detached.get(key) {
                    Some(d) => d.clone(),
                    None => continuation(self.active.get(key), tau_old, t, cur),
                };
                detached.insert(*key, cont);
            } else {
                active.insert(*key, Track { segs, source: "timeline" });
            }
        }
        self.active = active;
        self.detached = detached;
        self.head = Head { t0: t, tau0: to, dir: self.head.dir };
        self.last_tau = to;
    }

    pub fn play(&mut self, t: f64) {
        if matches!(self.mode, Mode::Main | Mode::Segment { .. } | Mode::Out { .. }) {
            self.event(t, "ignored", "play: уже идёт");
            return;
        }
        self.active = self.base.clone();
        self.detached.clear();
        self.head = Head { t0: t, tau0: 0.0, dir: 1.0 };
        self.mode = Mode::Main;
        self.loop_on = self.def.repeat.is_some();
        self.loop_left = self.def.repeat.as_ref().and_then(|l| l.count);
        self.saved = None;
        self.last_tau = -1.0;
        self.set_state(t, RunState::In);
        if self.next_stop().is_none() {
            self.flush(t);
            self.head.dir = 0.0;
            self.mode = Mode::Finished;
            self.set_state(t, RunState::Finished);
        }
    }

    pub fn next(&mut self, t: f64) {
        self.flush(t);
        let tau = self.head.tau(t);
        let at_out = self.def.markers.get("out").is_some_and(|o| (o - tau).abs() < EPS);
        match self.mode {
            Mode::Main if self.head.dir == 0.0 && self.state == RunState::Hold && !at_out => {
                self.head = Head { t0: t, tau0: tau, dir: 1.0 };
                self.set_state(t, RunState::In);
            }
            // Выход из цикла — только изнутри него: ранний next, пока до цикла не дошли, не
            // отменяет будущий цикл молча.
            Mode::Main if self.loop_on && self.state == RunState::Loop => {
                self.loop_on = false;
                self.head = Head { t0: t, tau0: tau, dir: 1.0 };
                self.set_state(t, RunState::In);
            }
            _ => {
                let why = if at_out { "next: стоим на out, дальше только stop" } else { "next: нет остановки" };
                self.event(t, "ignored", why);
            }
        }
    }

    /// stop: прыжок на `out` и воспроизведение до следующего маркера. Возвращает, когда экземпляр будет снят.
    pub fn stop(&mut self, t: f64) -> f64 {
        match self.mode {
            Mode::Removed => return t,
            Mode::Out { end } => return self.head.t0 + (end - self.head.tau0),
            Mode::Loaded => {
                self.mode = Mode::Removed;
                self.set_state(t, RunState::Removed);
                return t;
            }
            _ => {}
        }
        self.flush(t);
        self.saved = None;
        match self.def.markers.get("out").copied() {
            Some(out) => {
                let end = self.def.next_marker_after(out).unwrap_or(self.def.end).max(out);
                self.jump(t, out);
                self.head.dir = 1.0;
                self.mode = Mode::Out { end };
                self.event(t, "marker", "out");
                self.set_state(t, RunState::Out);
                if end - out <= EPS {
                    self.on_stop(t, Stop::OutEnd);
                }
                t + (end - out)
            }
            None => {
                self.head.dir = 0.0;
                self.mode = Mode::Removed;
                self.set_state(t, RunState::Removed);
                t
            }
        }
    }

    /// Сегмент команды: от маркера до следующего, потом возврат туда, где был.
    pub fn segment(&mut self, t: f64, marker: &str) -> Result<(), String> {
        let Some(&m) = self.def.markers.get(marker) else {
            let known = self.def.markers.keys().cloned().collect::<Vec<_>>().join(", ");
            return Err(format!("play({marker:?}): маркера нет; маркеры: {known}"));
        };
        if !matches!(self.mode, Mode::Main | Mode::Finished | Mode::Segment { .. }) {
            self.event(t, "ignored", &format!("play({marker}): экземпляр не в эфире"));
            return Ok(());
        }
        self.flush(t);
        if self.saved.is_none() {
            let tau = self.head.tau(t);
            self.saved = Some(Saved { tau, dir: self.head.dir, loop_on: self.loop_on, loop_left: self.loop_left, mode: self.mode, state: self.state });
        }
        let end = self.def.next_marker_after(m).unwrap_or(self.def.end).max(m);
        self.jump(t, m);
        self.head.dir = 1.0;
        self.mode = Mode::Segment { end };
        self.event(t, "marker", marker);
        self.set_state(t, RunState::Segment);
        if end - m <= EPS {
            self.on_stop(t, Stop::SegmentEnd);
        }
        Ok(())
    }

    /// Пометка для дампа: `timeline loop τ=2.31`, `timeline hold τ=0.7 на ready`.
    pub fn describe(&self, t: f64) -> crate::motion::Note {
        let tau = self.head.tau(t);
        // τ — время; шум f64 от сложения отрезков не нужен ни в тексте, ни в JSON.
        crate::motion::Note::Timeline { state: self.state, tau: (tau * 1e6).round() / 1e6, marker: self.marker_at(tau) }
    }
}

fn remaining(e: Easing, left: f64) -> Easing {
    match e {
        Easing::Tween { curve, .. } => Easing::Tween { duration: left.max(0.0), curve },
        spring => spring,
    }
}

/// Движение канала, которое было в момент прыжка, — дальше во времени сцены.
fn continuation(track: Option<&Track>, tau: f64, t: f64, cur: V) -> Track {
    let seg = track.and_then(|tr| tr.active(tau)).filter(|s| tau < s.start + s.dur);
    let seg = match seg {
        Some(s) => Seg::new(t, cur, s.to, remaining(s.easing, s.start + s.dur - tau), 0.0, false),
        None => Seg::new(t, cur, cur, Easing::Tween { duration: 0.0, curve: EASE_IN_OUT }, 0.0, false),
    };
    Track { segs: vec![seg], source: "timeline" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn def(v: Value) -> TimelineDef {
        let theme = BTreeMap::new();
        parse(&v, &mut Compiler { theme: &theme, notes: Vec::new() }).unwrap()
    }

    fn lin(d: f64) -> Value {
        json!({ "kind": "tween", "duration": d, "ease": "linear" })
    }

    #[test]
    fn fractional_loop_count_is_an_error() {
        let theme = BTreeMap::new();
        let v = json!({
            "timeline": "Card",
            "keys": [],
            "markers": { "loop": 1, "back": 2 },
            "loop": { "from": "loop", "to": "back", "count": 2.5 },
        });
        let errors = parse(&v, &mut Compiler { theme: &theme, notes: Vec::new() }).unwrap_err();
        assert!(errors.iter().any(|e| e.contains("loop.count")), "{errors:?}");
    }

    /// Ключи: opacity 0→1 за [0, 1], затем 1→0.5 в цикле [2, 3], out: 1→0 за [4, 5].
    fn run() -> Run {
        let d = def(json!({
            "timeline": "Card",
            "keys": [
                { "t": 0, "part": "root", "targets": { "opacity": [0, 1] }, "easing": lin(1.0) },
                { "t": 2, "part": "root", "targets": { "opacity": 0.5 }, "easing": lin(1.0) },
                { "t": 4, "part": "root", "targets": { "opacity": 0 }, "easing": lin(1.0) },
            ],
            "markers": { "ready": 1.5, "loop": 2, "back": 3, "out": 4 },
            "loop": { "from": "loop", "to": "back", "mode": "pingpong" },
            "holds": ["ready"],
            "events": [{ "t": 0.5, "name": "half" }],
        }));
        let nodes = BTreeMap::from([("root".to_string(), 7)]);
        Run::new("Card#1".into(), 7, d, &nodes, |_, _| V::N(1.0))
    }

    fn drive(r: &mut Run, until: f64) {
        while let Some((t, stop)) = r.next_stop() {
            if t > until {
                break;
            }
            r.on_stop(t, stop);
        }
        r.flush(until);
    }

    fn op(r: &Run, t: f64) -> f64 {
        r.value((7, Channel::Opacity), t).unwrap().n()
    }

    #[test]
    fn play_holds_then_next_loops_pingpong() {
        let mut r = run();
        assert_eq!(op(&r, 0.0), 0.0);
        r.play(0.0);
        drive(&mut r, 1.7);
        assert_eq!(r.state, RunState::Hold);
        assert!((r.tau(1.7) - 1.5).abs() < 1e-9);
        assert_eq!(op(&r, 1.7), 1.0);
        r.next(2.0);
        drive(&mut r, 4.0);
        // Вперёд до 3 (τ: 1.5 → 3 за 1.5 с, к моменту 3.5), затем назад: в 4.0 τ = 2.5.
        assert_eq!(r.state, RunState::Loop);
        assert!((r.tau(4.0) - 2.5).abs() < 1e-9, "{}", r.tau(4.0));
        assert!((op(&r, 4.0) - 0.75).abs() < 1e-9);
        // Назад до 2, снова вперёд: в 5.0 τ = 2.5.
        drive(&mut r, 5.0);
        assert!((r.tau(5.0) - 2.5).abs() < 1e-9, "{}", r.tau(5.0));
        // next выходит из цикла: дальше вперёд до out и стоп там.
        r.next(5.0);
        drive(&mut r, 7.0);
        assert_eq!(r.state, RunState::Hold);
        assert!((r.tau(7.0) - 4.0).abs() < 1e-9);
    }

    #[test]
    fn events_are_ordered_and_marked() {
        let mut r = run();
        r.play(0.0);
        drive(&mut r, 1.6);
        let names: Vec<String> = r.events.iter().map(|e| format!("{}@{}:{}", e.kind, e.t, e.name)).collect();
        assert_eq!(names, ["state@0:in", "emit@0.5:half", "marker@1.5:ready", "state@1.5:hold"]);
    }

    #[test]
    fn stop_mid_in_continues_from_current_value() {
        let mut r = run();
        r.play(0.0);
        drive(&mut r, 0.4);
        assert!((op(&r, 0.4) - 0.4).abs() < 1e-9);
        let gone = r.stop(0.4);
        // После out маркеров нет: воспроизведение идёт до конца таймлайна (τ=5), то есть 1 с.
        assert!((gone - 1.4).abs() < 1e-9, "{gone}");
        // out идёт от текущих 0.4, а не от объявленной единицы.
        assert!((op(&r, 0.4) - 0.4).abs() < 1e-9);
        assert!((op(&r, 0.9) - 0.2).abs() < 1e-9);
        drive(&mut r, 1.5);
        assert!(r.removed());
    }

    #[test]
    fn segment_returns_to_where_it_was() {
        let mut r = run();
        r.play(0.0);
        drive(&mut r, 1.6);
        r.segment(1.6, "loop").unwrap();
        assert_eq!(r.state, RunState::Segment);
        drive(&mut r, 3.0);
        // Сегмент loop..back длится 1 с, затем возврат к hold на ready.
        assert_eq!(r.state, RunState::Hold);
        assert!((r.tau(3.0) - 1.5).abs() < 1e-9);
        assert!(r.segment(3.0, "nope").is_err());
    }
}
