// Реализация @kine/core. Публичный контракт — spec/kine.d.ts: публичные функции типизированы
// его типами, а conformance.ts проверяет, что каждый экспорт спеки здесь есть и совместим.

import type * as S from '@kine/core';
import { host } from './host';
import { batch, computed, effect, runView, signal, untrack, type Accessor } from './signals';

export { batch, computed, effect, signal, untrack };

type Props = Record<string, any>;
export type KineElement = { readonly __kine: 'element'; readonly id: number };

class KineError extends Error {
  constructor(message: string, src?: string) {
    super(src ? `${message}\n  at ${src}` : message);
    this.name = 'KineError';
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

export const timeline: typeof S.timeline = (target, build) => {
  const at = (t: number, part: string, targets: S.AnimTargets, easing?: S.Easing) => brand<never>({ t, part, targets, easing });
  return brand({ timeline: (target as unknown as { kineName: string }).kineName, ...build({ at }) });
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
      if (!isSpan(c)) return { text: String(read(c as string)) };
      const inner = flat(c.children).filter((x) => !isEmpty(x));
      for (const x of inner) {
        if (isElement(x) || isSpan(x)) throw new KineError('Span не вкладывается: внутри только строки и аксессоры', c.src);
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

const counters = new Map<string, number>();

/** Экземпляры компонентов: команда сценария `update` меняет их пропсы. */
type Instance = { component: string; name: string; set(key: string, value: unknown): void; src?: string };
const instances: Instance[] = [];

export const component: typeof S.component = (def) => {
  if (typeof def.name !== 'string') throw new KineError('component(): нужно поле name');
  const C = (p: Props): KineElement => {
    const n = (counters.get(def.name) ?? 0) + 1;
    counters.set(def.name, n);
    const inst = `${def.name}#${n}`;

    // Каждый пропс — ячейка: снаружи в неё приходит значение или аксессор, а команда
    // `update` заменяет содержимое. Внутри компонента пропс всегда аксессор.
    const accessors: Props = {};
    const slots: Record<string, S.Signal<{ v: unknown }>> = {};
    for (const k of Object.keys(def.props ?? {})) {
      if (!(k in p)) throw new KineError(`${def.name}: не передан пропс "${k}"`, p.__src);
      const slot = signal<{ v: unknown }>({ v: p[k] });
      slots[k] = slot;
      accessors[k] = () => {
        const { v } = slot();
        return typeof v === 'function' ? (v as Accessor<unknown>)() : v;
      };
    }
    instances.push({
      component: def.name,
      name: inst,
      src: p.__src,
      set(key, value) {
        if (!(key in slots)) throw new KineError(`update: у ${def.name} нет пропса "${key}"`, p.__src);
        slots[key].set({ v: value });
      },
    });
    const parts: Props = {};
    for (const name of def.parts ?? []) parts[name] = { $part: name, inst };

    const root = runView(
      () =>
        host.emit({
          op: 'diag',
          level: 'warn',
          rule: 'signal.untracked',
          node: inst,
          source: p.__src,
          message: `${def.name}.view читает сигнал вне реактивного контекста: значение не будет обновляться`,
          hint: 'оберни чтение в аксессор () => …, on(signal, layer) или when={signal}; если так задумано — untrack(() => …)',
        }),
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

type CueItem = { t: number; cmd: string; target?: string; props?: Record<string, unknown> };

export const cues: typeof S.cues = (scene, build) => {
  const at = (t: number, cmd: string, target?: { kineName: string }, props?: Record<string, unknown>): CueItem => {
    if (typeof t !== 'number' || !(t >= 0)) throw new KineError(`cues: время должно быть числом ≥ 0, получено ${t}`);
    if (cmd === 'update' && !target?.kineName) throw new KineError('cues: update требует компонент: at(t, "update", LowerThird, { ... })');
    return { t, cmd, target: target?.kineName, props };
  };
  return brand({ $cues: true, scene, items: build(brand({ at })) });
};

/** Команда `update` из ядра: новые значения пропсов всех экземпляров компонента. */
(globalThis as any).__kine_update = (json: string) => {
  const { target, props } = JSON.parse(json) as { target: string; props: Record<string, unknown> };
  const found = instances.filter((i) => i.component === target);
  if (!found.length) throw new KineError(`update: в сцене нет экземпляров ${target}`);
  batch(() => {
    for (const i of found) for (const [k, v] of Object.entries(props ?? {})) i.set(k, v);
  });
  host.flush();
};

export function mount(entry: Props) {
  const scene = entry.$cues ? entry.scene : entry;
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
    () =>
      host.emit({
        op: 'diag',
        level: 'warn',
        rule: 'signal.untracked',
        node: 'stage',
        message: 'scene.root читает сигнал вне реактивного контекста: значение не будет обновляться',
        hint: 'передавай сигнал в пропс целиком (live={live}), а не его значение (live={live()})',
      }),
    () => scene.root(),
  );
  if (!isElement(root)) throw new KineError('scene.root должен вернуть элемент');
  host.insert(stage, root.id);
  if (entry.$cues) host.emit({ op: 'cues', items: entry.items });
  host.flush();
}

const notYet = (name: string) => (): never => {
  throw new KineError(`source.${name} появится в M6`);
};

export const source: typeof S.source = { json: notYet('json'), ws: notYet('ws'), osc: notYet('osc') };
