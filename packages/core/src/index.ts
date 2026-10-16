// Реализация @kine/core. Публичный контракт — spec/kine.d.ts: публичные функции типизированы
// его типами, а conformance.ts проверяет, что каждый экспорт спеки здесь есть и совместим.

import type * as S from '@kine/core';
import { host, type PropInfo } from './host';
import { batch, computed, effect, runView, signal, untrack, type Accessor } from './signals';

export { batch, computed, effect, signal, untrack };

type Props = Record<string, any>;
export type KineElement = { readonly __kine: 'element'; readonly id: number };

/** Ошибка API kine: место в TSX ядро берёт из `src`, стек бандла агенту не нужен. */
class KineError extends Error {
  readonly src?: string;
  constructor(message: string, src?: string) {
    super(message);
    this.name = 'KineError';
    this.src = src;
  }
}

const isElement = (v: unknown): v is KineElement => !!v && (v as KineElement).__kine === 'element';
type SpanDesc = { __kine: 'span'; look?: unknown; children?: unknown; src?: string };
const isSpan = (v: unknown): v is SpanDesc => !!v && (v as SpanDesc).__kine === 'span';
const isEmpty = (c: unknown) => c === null || c === undefined || c === false || c === true;
const read = <T>(v: T | Accessor<T>): T => (typeof v === 'function' ? (v as Accessor<T>)() : v);

/** Брендированные типы спеки (Motion, OnLayer, PartRef…) в рантайме — обычные объекты. */
const brand = <T>(v: unknown): T => v as T;

function flat(v: unknown, out: unknown[] = []): unknown[] {
  if (Array.isArray(v)) for (const x of v) flat(x, out);
  else out.push(v);
  return out;
}

// ─── Значения ───────────────────────────────────────────────────────────────

export const pct: typeof S.pct = (v) => ({ pct: v });

export const hex: typeof S.hex = (v, alpha = 1) => {
  const m = /^#([0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/i.exec(v);
  if (!m) throw new KineError(`hex(): ожидается #RGB, #RRGGBB или #RRGGBBAA, получено "${v}"`);
  let h = m[1];
  if (h.length === 3) h = [...h].map((c) => c + c).join('');
  const n = (i: number) => parseInt(h.slice(i, i + 2), 16);
  const a = h.length === 8 ? (n(6) / 255) * alpha : alpha;
  return { r: n(0), g: n(2), b: n(4), a };
};

export const oklch: typeof S.oklch = (l, c, h, alpha = 1) => ({ oklch: [l, c, h] as const, a: alpha });

export const on: typeof S.on = (cond, layer) => brand({ $on: cond, layer });

// ─── Раскладка ──────────────────────────────────────────────────────────────

export const row: typeof S.row = (o = {}) => ({ kind: 'row', ...o });
export const column: typeof S.column = (o = {}) => ({ kind: 'column', ...o });
export const grid: typeof S.grid = (o) => ({ kind: 'grid', ...o });
export const stack: typeof S.stack = () => ({ kind: 'stack' });

// ─── Движение: только данные, исполняет ядро ────────────────────────────────

export const tween: typeof S.tween = (duration, ease = 'inOut') => ({ kind: 'tween', duration, ease });

const springOf = (o: { stiffness: number; damping: number; mass?: number }): S.Spring => ({
  kind: 'spring',
  stiffness: o.stiffness,
  damping: o.damping,
  mass: o.mass ?? 1,
});
export const spring: typeof S.spring = Object.assign(springOf, {
  snappy: springOf({ stiffness: 400, damping: 30 }),
  gentle: springOf({ stiffness: 120, damping: 20 }),
  bouncy: springOf({ stiffness: 300, damping: 12 }),
});

/** Оценка длительности для seq/par; точное время пружин считает ядро. */
const easingDuration = (e?: S.Easing) => (!e ? 0.3 : e.kind === 'tween' ? e.duration : 0.6);
const motion = (data: Props, duration: number) => brand<S.Motion>({ ...data, duration });

export const animate: typeof S.animate = (targets, easing) =>
  motion({ motion: 'animate', targets, easing: easing ?? tween(0.3) }, easingDuration(easing));
export const fade: typeof S.fade = (range, easing) => animate({ opacity: range }, easing);
export const slide: typeof S.slide = (o, easing) => animate(o, easing);
export const seq: typeof S.seq = (...items) => motion({ motion: 'seq', items }, items.reduce((t, m) => t + m.duration, 0));
export const par: typeof S.par = (...items) => motion({ motion: 'par', items }, Math.max(0, ...items.map((m) => m.duration)));
export const delay: typeof S.delay = (seconds, m) => motion({ motion: 'delay', seconds, item: m }, seconds + m.duration);
export const pulse: typeof S.pulse = (o) => motion({ motion: 'pulse', ...o }, o.period);
export const stagger: typeof S.stagger = (step, m, o = {}) => motion({ motion: 'stagger', step, item: m, reverse: o.reverse ?? false }, m.duration);

// ─── Числа и часы: значение ведёт ядро, JS сообщает только смену данных ─────

type DynamicDesc = { __kine: 'dynamic'; describe: () => Record<string, unknown> };
const isDynamic = (v: unknown): v is DynamicDesc => !!v && (v as DynamicDesc).__kine === 'dynamic';

export const count: typeof S.count = (value, o = {}) =>
  brand({
    __kine: 'dynamic',
    describe: () => ({ kind: 'count', value: read(value), easing: o.easing ?? tween(0.6, 'out'), format: o.format ?? '0' }),
  });

export const clock: typeof S.clock = (o) =>
  brand({
    __kine: 'dynamic',
    describe: () => ({
      kind: 'clock',
      from: read(o.from),
      rate: o.rate ?? 1,
      running: read(o.running ?? true),
      format: o.format ?? 'mm:ss',
      min: o.min,
      max: o.max,
    }),
  });

export const timeline: typeof S.timeline = (target, build) => {
  const at = (t: number, part: string, targets: S.AnimTargets, easing?: S.Easing) => brand<never>({ t, part, targets, easing });
  const emit = (t: number, name: string) => brand<S.TimelineEmit>({ t, name });
  return brand({ timeline: (target as unknown as { kineName: string }).kineName, ...build({ at, emit }) });
};

// ─── Тема ───────────────────────────────────────────────────────────────────

const isLeaf = (v: any) =>
  v === null || typeof v !== 'object' || 'r' in v || 'oklch' in v || 'family' in v || Array.isArray(v);

function flattenTheme(values: unknown): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  const walk = (v: any, path: string[]) => {
    if (isLeaf(v)) out[path.join('.')] = v;
    else for (const k of Object.keys(v)) walk(v[k], [...path, k]);
  };
  walk(values, []);
  return out;
}

export const defineTheme: typeof S.defineTheme = (values) => {
  const tokens = (v: any, path: string[]): any => {
    if (isLeaf(v)) return { $t: path.join('.') };
    const out: Props = {};
    for (const k of Object.keys(v)) out[k] = tokens(v[k], [...path, k]);
    return out;
  };
  host.emit({ op: 'theme', values: flattenTheme(values) });
  return brand(Object.assign(tokens(values, []), { __values: values }));
};

export const setTheme: typeof S.setTheme = (_theme, values) => {
  host.emit({ op: 'theme', values: flattenTheme(values) });
};

// ─── look ───────────────────────────────────────────────────────────────────

type Layer = { label: string; props: Props };

function flattenLook(input: unknown, label: string, out: Layer[]): Layer[] {
  if (input === false || input === null || input === undefined) return out;
  if (Array.isArray(input)) {
    input.forEach((item, i) => flattenLook(item, `${label}[${i}]`, out));
  } else if (typeof input === 'object' && '$on' in (input as Props)) {
    const { $on, layer } = input as { $on: Accessor<boolean>; layer: unknown };
    if ($on()) flattenLook(layer, `${label} on`, out);
  } else if (typeof input === 'object') {
    out.push({ label, props: input as Props });
  } else {
    throw new KineError(`look: ожидается объект, массив или on(...), получено ${typeof input}`);
  }
  return out;
}

// ─── Узлы ───────────────────────────────────────────────────────────────────

const PLACEMENT = ['anchor', 'size', 'min', 'max', 'grow', 'shrink', 'aspect', 'cell'] as const;
const STATIC = ['layout', 'fit', 'must', 'presence', 'src', 'path', 'stroke', 'fill'] as const;

function applyPlacement(id: number, p: Props) {
  for (const k of PLACEMENT) if (p[k] !== undefined) host.set(id, k, p[k]);
  if (p.when !== undefined) effect(() => host.set(id, 'hidden', !read(p.when)));
}

function createNode(kind: string, p: Props): number {
  const id = host.create(kind, p.__src);
  applyPlacement(id, p);
  for (const k of STATIC) if (p[k] !== undefined) host.set(id, k, p[k]);
  if (p.else !== undefined) {
    if (!Array.isArray(p.else)) throw new KineError('else: ожидается массив шагов', p.__src);
    // Слои look в шагах уплощаются сразу: ядро применяет их как обычные слои.
    const steps = p.else.map((step: Props, i: number) =>
      step.look === undefined ? step : { ...step, look: flattenLook(step.look, `else[${i}].look`, []) },
    );
    host.set(id, 'else', steps);
  }
  if (p.name !== undefined) {
    if (!p.name || typeof p.name !== 'object' || !('$part' in p.name)) {
      throw new KineError('name принимает часть компонента (part.x), а не строку', p.__src);
    }
    host.set(id, 'part', { inst: p.name.inst, part: p.name.$part });
  }
  if (p.look !== undefined) effect(() => host.set(id, 'look', flattenLook(read(p.look), 'look', [])));
  return id;
}

const element = (id: number): KineElement => ({ __kine: 'element', id });

export function Box(p: Props): KineElement {
  const id = createNode('box', p);
  for (const c of flat(p.children)) {
    if (isEmpty(c)) continue;
    if (isSpan(c)) throw new KineError('Span бывает только внутри <Text>', c.src ?? p.__src);
    if (!isElement(c)) {
      throw new KineError(`Box принимает только элементы; текст оборачивай в <Text>. Получено: ${JSON.stringify(c)}`, p.__src);
    }
    host.insert(id, c.id);
  }
  return element(id);
}

export function Text(p: Props): KineElement {
  const id = createNode('text', p);
  const parts = flat(p.children).filter((c) => !isEmpty(c));
  for (const c of parts) {
    if (isElement(c)) throw new KineError('внутри Text допустимы только строки, аксессоры и <Span>', p.__src);
  }
  effect(() => {
    const runs = parts.map((c, i) => {
      if (isDynamic(c)) return { text: '', dynamic: c.describe() };
      if (!isSpan(c)) return { text: String(read(c as string)) };
      const inner = flat(c.children).filter((x) => !isEmpty(x));
      for (const x of inner) {
        if (isElement(x) || isSpan(x)) throw new KineError('Span не вкладывается: внутри только строки и аксессоры', c.src);
        if (isDynamic(x)) throw new KineError('count() и clock() пишутся прямо в <Text>, не внутри Span', c.src);
      }
      const text = inner.map((x) => String(read(x as string))).join('');
      const look = c.look === undefined ? undefined : flattenLook(read(c.look), `span[${i}]`, []);
      return { text, look, src: c.src };
    });
    host.runs(id, runs);
  });
  return element(id);
}

export function Image(p: Props): KineElement {
  return element(createNode('image', p));
}

export function Shape(p: Props): KineElement {
  return element(createNode('shape', p));
}

/** Кусок текста со своим стилем; живёт только внутри <Text>. */
export function Span(p: Props): KineElement {
  return { __kine: 'span', look: p.look, children: p.children, src: p.__src } as unknown as KineElement;
}

// ─── Компоненты ─────────────────────────────────────────────────────────────

export const prop: typeof S.prop = () => brand({ $prop: true });

type PropKind = 'string' | 'number' | 'boolean' | 'json';
const kindOf = (c: unknown): PropKind => (c === String ? 'string' : c === Number ? 'number' : c === Boolean ? 'boolean' : 'json');
const describeProps = (schema: Props | undefined): PropInfo[] =>
  Object.keys(schema ?? {}).map((name) => ({ name, type: kindOf(schema![name]) }));
const fitsKind = (v: unknown, kind: PropKind) =>
  kind === 'json' || (kind === 'number' ? typeof v === 'number' && Number.isFinite(v) : typeof v === kind);
const show = (v: unknown) => JSON.stringify(v) ?? String(v);

/**
 * Ячейки пропсов компонента или сцены. Значение или аксессор лежит в сигнале в обёртке `{ v }`,
 * чтобы функция хранилась как значение; `set` заменяет содержимое, как команда `update`.
 * Снаружи пропс всегда аксессор.
 */
function propSlots(initial: Props, owner: string, src?: string) {
  const accessors: Props = {};
  const slots: Record<string, S.Signal<{ v: unknown }>> = {};
  for (const k of Object.keys(initial)) {
    const slot = signal<{ v: unknown }>({ v: initial[k] });
    slots[k] = slot;
    accessors[k] = () => read(slot().v);
  }
  const known = Object.keys(slots);
  const list = known.length ? `есть: ${known.join(', ')}` : 'пропсов нет';
  const set = (key: string, value: unknown) => {
    if (!(key in slots)) throw new KineError(`update: у ${owner} нет пропса "${key}"; ${list}`, src);
    slots[key].set({ v: value });
  };
  return { accessors, set };
}

/** Диагностика чтения сигнала вне реактивного контекста — для runView. */
const untracked = (node: string, message: string, hint: string, source?: string) => () =>
  host.emit({ op: 'diag', level: 'warn', rule: 'signal.untracked', node, source, message, hint });

const counters = new Map<string, number>();

/** Экземпляры компонентов: команды сценария `update` и `invoke` адресуют их по имени компонента. */
type Instance = {
  component: string;
  name: string;
  src?: string;
  set(key: string, value: unknown): void;
  invoke(command: string, params: Record<string, unknown>): void;
};
const instances: Instance[] = [];

/** Команда движения экземпляру: ядро исполнит её в тот же момент, что и команду сценария. */
const motionCmd = (inst: string, cmd: string, extra: Record<string, unknown> = {}) => host.emit({ op: 'motion', inst, cmd, ...extra });

export const component: typeof S.component = (def) => {
  if (typeof def.name !== 'string') throw new KineError('component(): нужно поле name');
  const commandDefs = (def.commands ?? {}) as Record<string, { params?: Props }>;
  host.emit({
    op: 'component',
    name: def.name,
    props: describeProps(def.props),
    parts: [...(def.parts ?? [])],
    commands: Object.keys(commandDefs).map((name) => ({ name, params: describeProps(commandDefs[name].params) })),
  });
  const C = (p: Props): KineElement => {
    const n = (counters.get(def.name) ?? 0) + 1;
    counters.set(def.name, n);
    const inst = `${def.name}#${n}`;

    // Каждый пропс — ячейка: снаружи в неё приходит значение или аксессор.
    const initial: Props = {};
    for (const k of Object.keys(def.props ?? {})) {
      if (!(k in p)) throw new KineError(`${def.name}: не передан пропс "${k}"`, p.__src);
      initial[k] = p[k];
    }
    const { accessors, set } = propSlots(initial, def.name, p.__src);
    const commands = (def.commands ?? {}) as Record<string, { params: Props; run: (params: Props, ctx: unknown) => void }>;
    const ctx = {
      props: accessors,
      set: (patch: Props) => batch(() => Object.entries(patch).forEach(([k, v]) => set(k, v))),
      play: (marker?: string) => motionCmd(inst, 'play', marker === undefined ? {} : { marker }),
      next: () => motionCmd(inst, 'next'),
      stop: () => motionCmd(inst, 'stop'),
      emit: (name: string, data?: unknown) => motionCmd(inst, 'emit', { name, data }),
    };
    instances.push({
      component: def.name,
      name: inst,
      src: p.__src,
      set,
      invoke(command, params) {
        const c = commands[command];
        if (!c) {
          const known = Object.keys(commands).join(', ') || 'нет ни одной';
          throw new KineError(`invoke: у ${def.name} нет команды "${command}"; команды: ${known}`, p.__src);
        }
        for (const k of Object.keys(c.params ?? {})) {
          if (!(k in (params ?? {}))) throw new KineError(`invoke ${def.name}.${command}: не передан параметр "${k}"`, p.__src);
        }
        batch(() => c.run(params ?? {}, ctx));
      },
    });
    const parts: Props = {};
    for (const name of def.parts ?? []) parts[name] = { $part: name, inst };

    const root = runView(
      untracked(
        inst,
        `${def.name}.view читает сигнал вне реактивного контекста: значение не будет обновляться`,
        'оберни чтение в аксессор () => …, on(signal, layer) или when={signal}; если так задумано — untrack(() => …)',
        p.__src,
      ),
      () => def.view(brand(accessors), brand(parts)),
    );
    if (!isElement(root)) throw new KineError(`${def.name}.view должен вернуть один элемент`, p.__src);
    host.set(root.id, 'instance', { name: inst, src: p.__src });
    if (def.presence) host.set(root.id, 'presence', def.presence);
    applyPlacement(root.id, p);
    return root;
  };
  return brand(Object.assign(C, { kineName: def.name }));
};

// ─── Сцена и сценарий ───────────────────────────────────────────────────────

export const defineScene: typeof S.defineScene = (def) => def;

type CueItem = { t: number; cmd: string; target?: string; props?: Record<string, unknown>; command?: string };

const CUE_COMMANDS = ['play', 'stop', 'next', 'update', 'invoke'];

export const cues: typeof S.cues = (scene, build) => {
  const at = (t: number, cmd: string, target?: unknown, a?: unknown, b?: unknown): CueItem => {
    if (typeof t !== 'number' || !(t >= 0)) throw new KineError(`cues: время должно быть числом ≥ 0, получено ${t}`);
    if (!CUE_COMMANDS.includes(cmd)) throw new KineError(`cues: команда "${cmd}" не поддерживается; есть ${CUE_COMMANDS.join(', ')}`);
    const name = (target as { kineName?: string } | undefined)?.kineName;
    // update без компонента — пропсы самой сцены: at(t, 'update', { name: 'Иван' }).
    if (cmd === 'update' && !name) {
      if (!target || typeof target !== 'object') {
        throw new KineError('cues: update принимает компонент и его пропсы или пропсы сцены: at(t, "update", { ... })');
      }
      return { t, cmd, props: target as Record<string, unknown> };
    }
    if (cmd === 'invoke' && !name) throw new KineError('cues: invoke требует компонент: at(t, "invoke", Component, "команда", { ... })');
    if (cmd === 'invoke') return { t, cmd, target: name, command: a as string, props: b as Record<string, unknown> };
    return { t, cmd, target: name, props: a as Record<string, unknown> | undefined };
  };
  return brand({ $cues: true, scene, items: build(brand({ at })) });
};

/** Пропсы сцены — ячейки, как у компонента; `update` без компонента меняет их. */
let sceneSet: ((key: string, value: unknown) => void) | undefined;

/** Пропсы из CLI: `values` — из --props, `raw` — строки из --set, приводятся к типу схемы. */
type PropsInput = { values?: Record<string, unknown>; raw?: Record<string, string> };

function sceneProps(scene: Props): { accessors: Props; errors: string[] } {
  const input = ((globalThis as any).__kine_props ?? {}) as PropsInput;
  const schema: Props = scene.props ?? {};
  const defaults: Props = scene.defaults ?? {};
  const errors: string[] = [];
  const known = Object.keys(schema);
  const list = known.length ? `есть: ${known.join(', ')}` : 'у сцены нет пропсов';
  for (const k of [...Object.keys(input.values ?? {}), ...Object.keys(input.raw ?? {})]) {
    if (!(k in schema)) errors.push(`у сцены нет пропса "${k}"; ${list}`);
  }
  const values: Props = {};
  for (const k of known) {
    const kind = kindOf(schema[k]);
    const raw = input.raw && k in input.raw ? input.raw[k] : undefined;
    let v: unknown;
    if (raw !== undefined) {
      if (kind === 'string') v = raw;
      else if (kind === 'number') v = raw.trim() === '' ? NaN : Number(raw);
      else if (kind === 'boolean') v = raw === 'true' ? true : raw === 'false' ? false : raw;
      else {
        try {
          v = JSON.parse(raw);
        } catch {
          errors.push(`--set ${k}: ожидается JSON, получено ${raw}`);
          continue;
        }
      }
    } else if (input.values && k in input.values) v = input.values[k];
    else if (k in defaults) v = defaults[k];
    else {
      errors.push(`не задан обязательный пропс "${k}" (${kind}): --set ${k}=… или --props data.json`);
      continue;
    }
    if (!fitsKind(v, kind)) {
      errors.push(`пропс "${k}": ожидается ${kind}, получено ${raw ?? show(v)}`);
      continue;
    }
    values[k] = v;
  }
  const { accessors, set } = propSlots(values, 'сцены');
  sceneSet = set;
  host.emit({
    op: 'schema',
    props: known.map((name) => ({
      name,
      type: kindOf(schema[name]),
      ...(name in defaults ? { default: defaults[name] } : { required: true }),
    })),
    errors,
  });
  return { accessors, errors };
}

const instancesOf = (target: string, what: string) => {
  const found = instances.filter((i) => i.component === target);
  if (!found.length) throw new KineError(`${what}: в сцене нет экземпляров ${target}`);
  return found;
};

/** Команда `update` из ядра: новые значения пропсов всех экземпляров компонента. */
(globalThis as any).__kine_update = (json: string) => {
  const { target, props } = JSON.parse(json) as { target: string | null; props: Record<string, unknown> };
  // Без компонента — пропсы самой сцены.
  const setters = target === null ? [sceneSet!] : instancesOf(target, 'update').map((i) => i.set);
  batch(() => {
    for (const set of setters) for (const [k, v] of Object.entries(props ?? {})) set(k, v);
  });
  host.flush();
};

/** Команда `invoke` из ядра: команда компонента на всех его экземплярах. */
(globalThis as any).__kine_invoke = (json: string) => {
  const { target, command, params } = JSON.parse(json) as { target: string; command: string; params: Record<string, unknown> };
  for (const i of instancesOf(target, 'invoke')) i.invoke(command, params);
  host.flush();
};

export function mount(entry: Props) {
  const scene = entry.$cues ? entry.scene : entry;
  const { accessors, errors } = sceneProps(scene);
  if (errors.length) {
    // Неверные входные данные — ошибка вызова: сцену не строим, ядро покажет ошибки и схему.
    host.flush();
    return;
  }
  const stage = host.create('stage');
  host.emit({
    op: 'stage',
    id: stage,
    ref: scene.stage.ref,
    safe: scene.stage.safe,
    fps: scene.fps,
    fonts: scene.fonts ?? [],
    systemFonts: scene.systemFonts ?? true,
  });
  const root = runView(
    untracked(
      'stage',
      'scene.root читает сигнал вне реактивного контекста: значение не будет обновляться',
      'передавай сигнал в пропс целиком (live={live}), а не его значение (live={live()})',
    ),
    () => scene.root(accessors),
  );
  if (!isElement(root)) throw new KineError('scene.root должен вернуть элемент');
  host.insert(stage, root.id);
  if (scene.timelines?.length) host.emit({ op: 'timelines', items: scene.timelines });
  if (entry.$cues) host.emit({ op: 'cues', items: entry.items });
  host.flush();
}

const notYet = (name: string) => (): never => {
  throw new KineError(`source.${name} появится в M6`);
};

export const source: typeof S.source = { json: notYet('json'), ws: notYet('ws'), osc: notYet('osc') };
