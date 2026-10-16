// Реализация @keyfill/core. Публичный контракт — spec/keyfill.d.ts: публичные функции типизированы
// его типами, а conformance.ts проверяет, что каждый экспорт спеки здесь есть и совместим.

import type * as S from '@keyfill/core';
import { host } from './host';
import { coerce, descAll, fields, infos, prop, type PropDesc } from './props';
import { batch, computed, createRoot, effect, runView, signal, untrack, type Accessor } from './signals';

export { batch, computed, effect, prop, signal, untrack };

type Props = Record<string, any>;
export type KeyfillElement = { readonly __keyfill: 'element'; readonly id: number };

/** Ошибка API keyfill: место в TSX ядро берёт из `src`, стек бандла агенту не нужен. */
class KeyfillError extends Error {
  readonly src?: string;
  constructor(message: string, src?: string) {
    super(message);
    this.name = 'KeyfillError';
    this.src = src;
  }
}

const isElement = (v: unknown): v is KeyfillElement => !!v && (v as KeyfillElement).__keyfill === 'element';
type SpanDesc = { __keyfill: 'span'; look?: unknown; children?: unknown; src?: string };
const isSpan = (v: unknown): v is SpanDesc => !!v && (v as SpanDesc).__keyfill === 'span';
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
  if (!m) throw new KeyfillError(`hex(): ожидается #RGB, #RRGGBB или #RRGGBBAA, получено "${v}"`);
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

type DynamicDesc = { __keyfill: 'dynamic'; describe: () => Record<string, unknown> };
const isDynamic = (v: unknown): v is DynamicDesc => !!v && (v as DynamicDesc).__keyfill === 'dynamic';

export const count: typeof S.count = (value, o = {}) =>
  brand({
    __keyfill: 'dynamic',
    describe: () => ({ kind: 'count', value: read(value), easing: o.easing ?? tween(0.6, 'out'), format: o.format ?? '0' }),
  });

export const clock: typeof S.clock = (o) =>
  brand({
    __keyfill: 'dynamic',
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
  return brand({ timeline: (target as unknown as { keyfillName: string }).keyfillName, ...build({ at, emit }) });
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
    throw new KeyfillError(`look: ожидается объект, массив или on(...), получено ${typeof input}`);
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
    if (!Array.isArray(p.else)) throw new KeyfillError('else: ожидается массив шагов', p.__src);
    // Слои look в шагах уплощаются сразу: ядро применяет их как обычные слои.
    const steps = p.else.map((step: Props, i: number) =>
      step.look === undefined ? step : { ...step, look: flattenLook(step.look, `else[${i}].look`, []) },
    );
    host.set(id, 'else', steps);
  }
  if (p.name !== undefined) {
    if (!p.name || typeof p.name !== 'object' || !('$part' in p.name)) {
      throw new KeyfillError('name принимает часть компонента (part.x), а не строку', p.__src);
    }
    host.set(id, 'part', { inst: p.name.inst, part: p.name.$part });
  }
  if (p.look !== undefined) effect(() => host.set(id, 'look', flattenLook(read(p.look), 'look', [])));
  return id;
}

const element = (id: number): KeyfillElement => ({ __keyfill: 'element', id });

export function Box(p: Props): KeyfillElement {
  const id = createNode('box', p);
  // Дети — элементы и списки For; список вставляет свои элементы перед следующим ребёнком.
  const slots: (number | ListRegion)[] = [];
  for (const c of flat(p.children)) {
    if (isEmpty(c)) continue;
    if (isSpan(c)) throw new KeyfillError('Span бывает только внутри <Text>', c.src ?? p.__src);
    if (isFor(c)) {
      slots.push(new ListRegion(id, c));
      continue;
    }
    if (!isElement(c)) {
      throw new KeyfillError(`Box принимает только элементы и <For>; текст оборачивай в <Text>. Получено: ${JSON.stringify(c)}`, p.__src);
    }
    host.insert(id, c.id);
    slots.push(c.id);
  }
  slots.forEach((s, i) => {
    if (typeof s === 'number') return;
    s.anchor = () => {
      for (const next of slots.slice(i + 1)) {
        const first = typeof next === 'number' ? next : next.first();
        if (first !== undefined) return first;
      }
      return undefined;
    };
    s.start();
  });
  return element(id);
}

// ─── Списки по данным ───────────────────────────────────────────────────────

type ForDesc = {
  __keyfill: 'for';
  each: unknown;
  by?: (item: unknown, index: number) => unknown;
  render: (item: Accessor<unknown>, index: Accessor<number>) => unknown;
  src?: string;
};
const isFor = (v: unknown): v is ForDesc => !!v && (v as ForDesc).__keyfill === 'for';

/** Список элементов по данным: `<For each={p.players} by={(pl) => pl.id}>{(pl, i) => <Row … />}</For>`. */
export const For: typeof S.For = ((p: Props) => {
  if (typeof p.children !== 'function') throw new KeyfillError('For: ребёнок — функция (item, index) => <…/>', p.__src);
  if (p.by !== undefined && typeof p.by !== 'function') throw new KeyfillError('For: by — функция (item) => ключ', p.__src);
  return brand({ __keyfill: 'for', each: p.each, by: p.by, render: p.children, src: p.__src });
}) as typeof S.For;

type ListItem = { key: string; root: number; value: S.Signal<{ v: unknown }>; index: S.Signal<number>; dispose: () => void };

/**
 * Элементы `For` внутри контейнера. Ключ решает, что такое «тот же элемент»: тот же ключ с
 * новыми данными — обновление (узлы остаются, значения идут через transition), новый ключ —
 * монтирование и `in`, пропавший — `release`: ядро проигрывает `out` и удаляет узлы.
 */
class ListRegion {
  private items = new Map<string, ListItem>();
  private order: ListItem[] = [];
  anchor: () => number | undefined = () => undefined;

  constructor(
    private parent: number,
    private desc: ForDesc,
  ) {}

  first(): number | undefined {
    return this.order[0]?.root;
  }

  start() {
    effect(() => {
      const list = read(this.desc.each);
      if (!Array.isArray(list)) throw new KeyfillError(`For: each — массив, получено ${JSON.stringify(list)}`, this.desc.src);
      untrack(() => this.reconcile(list));
    });
  }

  private keyOf(v: unknown, i: number): string {
    if (this.desc.by) return String(this.desc.by(v, i));
    if (typeof v === 'string' || typeof v === 'number') return String(v);
    throw new KeyfillError('For: у элементов-объектов нужен ключ: by={(item) => item.id}', this.desc.src);
  }

  private create(key: string, v: unknown, i: number): ListItem {
    const value = signal({ v });
    const index = signal(i);
    const mounted = createRoot(() =>
      runView(
        untracked(
          `For[${key}]`,
          'элемент For читает сигнал вне реактивного контекста: значение не будет обновляться',
          'данные элемента — аксессор: (item) => <Text>{() => item().name}</Text>',
          this.desc.src,
        ),
        () => this.desc.render(() => value().v, index),
      ),
    );
    if (!isElement(mounted.value)) {
      mounted.dispose();
      throw new KeyfillError('For: функция элемента должна вернуть один элемент', this.desc.src);
    }
    host.set(mounted.value.id, 'key', key);
    return { key, root: mounted.value.id, value, index, dispose: mounted.dispose };
  }

  private reconcile(list: readonly unknown[]) {
    const next: ListItem[] = [];
    const seen = new Set<string>();
    list.forEach((v, i) => {
      const key = this.keyOf(v, i);
      if (seen.has(key)) {
        const message = `For: ключ "${key}" повторяется; показан первый элемент с ним`;
        host.emit({ op: 'diag', level: 'warn', rule: 'list.duplicate-key', node: 'For', source: this.desc.src, message, hint: 'ключ by должен быть уникален в списке: id, а не имя' });
        return;
      }
      seen.add(key);
      const it = this.items.get(key);
      if (it) {
        it.value.set({ v });
        it.index.set(i);
        next.push(it);
      } else {
        const created = this.create(key, v, i);
        this.items.set(key, created);
        next.push(created);
      }
    });
    for (const [key, it] of this.items) {
      if (seen.has(key)) continue;
      it.dispose();
      host.emit({ op: 'release', id: it.root });
      this.items.delete(key);
    }
    // Двигаются только новые и переставленные: элементы, сохранившие взаимный порядок, стоят,
    // и уходящие остаются там, где были, пока доигрывают out.
    const old = new Map(this.order.map((it, i) => [it, i]));
    const stable = increasing(next.map((it) => old.get(it) ?? -1));
    this.order = next;
    for (let i = next.length - 1; i >= 0; i--) {
      if (stable.has(i)) continue;
      host.insert(this.parent, next[i].root, i + 1 < next.length ? next[i + 1].root : this.anchor());
    }
  }
}

/** Номера элементов наибольшей возрастающей подпоследовательности `seq` (отрицательные пропускаются). */
function increasing(seq: number[]): Set<number> {
  const tails: number[] = [];
  const prev = new Array<number>(seq.length).fill(-1);
  seq.forEach((v, i) => {
    if (v < 0) return;
    let lo = 0;
    let hi = tails.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (seq[tails[mid]] < v) lo = mid + 1;
      else hi = mid;
    }
    if (lo > 0) prev[i] = tails[lo - 1];
    tails[lo] = i;
  });
  const out = new Set<number>();
  for (let i = tails.length ? tails[tails.length - 1] : -1; i >= 0; i = prev[i]) out.add(i);
  return out;
}

export function Text(p: Props): KeyfillElement {
  const id = createNode('text', p);
  const parts = flat(p.children).filter((c) => !isEmpty(c));
  for (const c of parts) {
    if (isElement(c)) throw new KeyfillError('внутри Text допустимы только строки, аксессоры и <Span>', p.__src);
  }
  effect(() => {
    const runs = parts.map((c, i) => {
      if (isDynamic(c)) return { text: '', dynamic: c.describe() };
      if (!isSpan(c)) return { text: String(read(c as string)) };
      const inner = flat(c.children).filter((x) => !isEmpty(x));
      for (const x of inner) {
        if (isElement(x) || isSpan(x)) throw new KeyfillError('Span не вкладывается: внутри только строки и аксессоры', c.src);
        if (isDynamic(x)) throw new KeyfillError('count() и clock() пишутся прямо в <Text>, не внутри Span', c.src);
      }
      const text = inner.map((x) => String(read(x as string))).join('');
      const look = c.look === undefined ? undefined : flattenLook(read(c.look), `span[${i}]`, []);
      return { text, look, src: c.src };
    });
    host.runs(id, runs);
  });
  return element(id);
}

export function Image(p: Props): KeyfillElement {
  return element(createNode('image', p));
}

export function Shape(p: Props): KeyfillElement {
  return element(createNode('shape', p));
}

/** Кусок текста со своим стилем; живёт только внутри <Text>. */
export function Span(p: Props): KeyfillElement {
  return { __keyfill: 'span', look: p.look, children: p.children, src: p.__src } as unknown as KeyfillElement;
}

// ─── Компоненты ─────────────────────────────────────────────────────────────

/** Ошибки проверки значений — одним исключением с местом в TSX. */
function valid<T>(what: string, src: string | undefined, run: (errors: string[]) => T): T {
  const errors: string[] = [];
  const out = run(errors);
  if (errors.length) throw new KeyfillError(`${what}: ${errors.join('; ')}`, src);
  return out;
}

/** Значения и аксессоры: значения проверяются по типу, аксессоры — нет (их проверил tsc). */
function split(given: Props): { plain: Props; fns: Props } {
  const plain: Props = {};
  const fns: Props = {};
  for (const [k, v] of Object.entries(given)) (typeof v === 'function' ? fns : plain)[k] = v;
  return { plain, fns };
}

/**
 * Ячейки пропсов компонента или сцены. Значение или аксессор лежит в сигнале в обёртке `{ v }`,
 * чтобы функция хранилась как значение. `update` — частичное обновление, как команда `update`:
 * значения проверяются по типам и применяются все вместе, только если ошибок нет.
 * Снаружи пропс всегда аксессор.
 */
function propSlots(schema: Record<string, PropDesc>, initial: Props, owner: string, src?: string) {
  const accessors: Props = {};
  const slots: Record<string, S.Signal<{ v: unknown }>> = {};
  for (const k of Object.keys(initial)) {
    const slot = signal<{ v: unknown }>({ v: initial[k] });
    slots[k] = slot;
    accessors[k] = () => read(slot().v);
  }
  const update = (patch: Props) => {
    const values = valid(`update ${owner}`, src, (errors) => {
      const { plain, fns } = split(patch ?? {});
      for (const k of Object.keys(fns)) if (!(k in schema)) fields(schema, { [k]: 0 }, '', errors, false, {}, owner);
      return { ...fields(schema, plain, '', errors, false, {}, owner), ...fns };
    });
    batch(() => Object.entries(values).forEach(([k, v]) => slots[k].set({ v })));
  };
  /** Текущие значения — для снимка состояния канала. */
  const current = () => untrack(() => Object.fromEntries(Object.keys(slots).map((k) => [k, accessors[k]()])));
  return { accessors, update, current };
}

/** Начальные пропсы экземпляра: переданные значения, аксессоры и значения по умолчанию. */
function initialProps(schema: Record<string, PropDesc>, given: Props, owner: string, src?: string): Props {
  return valid(owner, src, (errors) => {
    const { plain, fns } = split(Object.fromEntries(Object.entries(given).filter(([k]) => k in schema)));
    const out: Props = { ...fields(schema, plain, '', errors, false, {}, owner), ...fns };
    for (const k of Object.keys(schema)) {
      if (k in out) continue;
      if (schema[k].default !== undefined) out[k] = schema[k].default;
      else errors.push(`не передан пропс "${k}"`);
    }
    return out;
  });
}

/** Диагностика чтения сигнала вне реактивного контекста — для runView. */
const untracked = (node: string, message: string, hint: string, source?: string) => () =>
  host.emit({ op: 'diag', level: 'warn', rule: 'signal.untracked', node, source, message, hint });

const counters = new Map<string, number>();

/** Экземпляры компонентов: команды сценария адресуют их по имени компонента, канал — по слою. */
type Instance = {
  component: string;
  name: string;
  src?: string;
  root: number;
  update(patch: Props): void;
  current(): Props;
  invoke(command: string, params: Record<string, unknown>): void;
  /** Ошибки команды до исполнения: канал отклоняет её, ничего не меняя. */
  checkUpdate(patch: Props): string[];
  checkInvoke(command: string, params: Record<string, unknown>): string[];
};
let instances: Instance[] = [];

/** Команда движения экземпляру: ядро исполнит её в тот же момент, что и команду сценария. */
const motionCmd = (inst: string, cmd: string, extra: Record<string, unknown> = {}) => host.emit({ op: 'motion', inst, cmd, ...extra });

export const component: typeof S.component = (def) => {
  if (typeof def.name !== 'string') throw new KeyfillError('component(): нужно поле name');
  const commandDefs = (def.commands ?? {}) as Record<string, { params?: Props; run: (params: Props, ctx: unknown) => void }>;
  const schema = descAll(def.props, `${def.name}: пропс`);
  const params: Record<string, Record<string, PropDesc>> = {};
  for (const name of Object.keys(commandDefs)) params[name] = descAll(commandDefs[name].params, `${def.name}.${name}: параметр`);
  host.emit({
    op: 'component',
    name: def.name,
    props: infos(schema),
    parts: [...(def.parts ?? [])],
    commands: Object.keys(commandDefs).map((name) => ({ name, params: infos(params[name]) })),
  });
  const C = (p: Props): KeyfillElement => {
    const n = (counters.get(def.name) ?? 0) + 1;
    counters.set(def.name, n);
    const inst = `${def.name}#${n}`;

    // Каждый пропс — ячейка: снаружи в неё приходит значение или аксессор.
    const initial = initialProps(schema, p, def.name, p.__src);
    const { accessors, update, current } = propSlots(schema, initial, def.name, p.__src);
    const ctx = {
      props: accessors,
      set: update,
      play: (marker?: string) => motionCmd(inst, 'play', marker === undefined ? {} : { marker }),
      next: () => motionCmd(inst, 'next'),
      stop: () => motionCmd(inst, 'stop'),
      emit: (name: string, data?: unknown) => motionCmd(inst, 'emit', { name, data }),
    };
    const instance: Instance = {
      component: def.name,
      name: inst,
      src: p.__src,
      root: 0,
      update,
      current,
      invoke(command, given) {
        const c = commandDefs[command];
        if (!c) {
          const known = Object.keys(commandDefs).join(', ') || 'нет ни одной';
          throw new KeyfillError(`invoke: у ${def.name} нет команды "${command}"; команды: ${known}`, p.__src);
        }
        const values = valid(`invoke ${def.name}.${command}`, p.__src, (errors) =>
          fields(params[command], given ?? {}, '', errors, true, {}, `команды ${command}`),
        );
        batch(() => c.run(values, ctx));
      },
      checkUpdate(patch) {
        const errors: string[] = [];
        fields(schema, patch ?? {}, '', errors, false, {}, def.name);
        return errors;
      },
      checkInvoke(command, given) {
        if (!commandDefs[command]) {
          return [`у ${def.name} нет команды "${command}"; команды: ${Object.keys(commandDefs).join(', ') || 'нет ни одной'}`];
        }
        const errors: string[] = [];
        fields(params[command], given ?? {}, '', errors, true, {}, `команды ${command}`);
        return errors;
      },
    };
    instances.push(instance);
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
    if (!isElement(root)) throw new KeyfillError(`${def.name}.view должен вернуть один элемент`, p.__src);
    instance.root = root.id;
    host.set(root.id, 'instance', { name: inst, src: p.__src });
    if (def.presence) host.set(root.id, 'presence', def.presence);
    applyPlacement(root.id, p);
    return root;
  };
  return brand(Object.assign(C, { keyfillName: def.name, keyfillSchema: schema }));
};

// ─── Сцена и сценарий ───────────────────────────────────────────────────────

export const defineScene: typeof S.defineScene = (def) => def;

type CueItem = { t: number; cmd: string; target?: string; props?: Record<string, unknown>; command?: string };

const CUE_COMMANDS = ['play', 'stop', 'next', 'update', 'invoke'];

export const cues: typeof S.cues = (scene, build) => {
  const at = (t: number, cmd: string, target?: unknown, a?: unknown, b?: unknown): CueItem => {
    if (typeof t !== 'number' || !(t >= 0)) throw new KeyfillError(`cues: время должно быть числом ≥ 0, получено ${t}`);
    if (!CUE_COMMANDS.includes(cmd)) throw new KeyfillError(`cues: команда "${cmd}" не поддерживается; есть ${CUE_COMMANDS.join(', ')}`);
    const name = (target as { keyfillName?: string } | undefined)?.keyfillName;
    // update без компонента — пропсы самой сцены: at(t, 'update', { name: 'Иван' }).
    if (cmd === 'update' && !name) {
      if (!target || typeof target !== 'object') {
        throw new KeyfillError('cues: update принимает компонент и его пропсы или пропсы сцены: at(t, "update", { ... })');
      }
      return { t, cmd, props: target as Record<string, unknown> };
    }
    if (cmd === 'invoke' && !name) throw new KeyfillError('cues: invoke требует компонент: at(t, "invoke", Component, "команда", { ... })');
    if (cmd === 'invoke') return { t, cmd, target: name, command: a as string, props: b as Record<string, unknown> };
    return { t, cmd, target: name, props: a as Record<string, unknown> | undefined };
  };
  return brand({ $cues: true, scene, items: build(brand({ at })) });
};

/** Пропсы сцены — ячейки, как у компонента; `update` без компонента меняет их. */
let sceneUpdate: ((patch: Props) => void) | undefined;

/** Пропсы из CLI: `values` — из --props, `raw` — строки из --set, приводятся к типу схемы. */
type PropsInput = { values?: Record<string, unknown>; raw?: Record<string, string> };

function sceneProps(scene: Props, input: PropsInput): { accessors: Props; errors: string[] } {
  const schema = descAll(scene.props, 'defineScene: пропс');
  const defaults: Props = scene.defaults ?? {};
  const errors: string[] = [];
  // Строки из --set приводятся к типу пропса; неизвестные имена ловит проверка ниже.
  const given: Props = { ...(input.values ?? {}) };
  for (const [k, raw] of Object.entries(input.raw ?? {})) {
    const c = k in schema ? coerce(schema[k], raw) : { value: raw };
    if (c.error) errors.push(`--set ${k}: ${c.error}`);
    else given[k] = c.value;
  }
  const before = errors.length;
  const values = fields(schema, given, '', errors, true, defaults, 'сцены');
  for (let i = before; i < errors.length; i++) {
    const m = /^не задан обязательный пропс "([^"]+)"/.exec(errors[i]);
    if (m) errors[i] += `: --set ${m[1]}=… или --props data.json`;
  }
  const { accessors, update } = propSlots(schema, values, 'сцены');
  sceneUpdate = update;
  host.emit({ op: 'schema', props: infos(schema, defaults), errors });
  return { accessors, errors };
}

const instancesOf = (target: string, what: string) => {
  const found = instances.filter((i) => i.component === target);
  if (!found.length) throw new KeyfillError(`${what}: в сцене нет экземпляров ${target}`);
  return found;
};

/** Команда `update` из ядра: новые значения пропсов всех экземпляров компонента. */
(globalThis as any).__keyfill_update = (json: string) => {
  const { target, props } = JSON.parse(json) as { target: string | null; props: Record<string, unknown> };
  // Без компонента — пропсы самой сцены.
  const updates = target === null ? [sceneUpdate!] : instancesOf(target, 'update').map((i) => i.update);
  batch(() => updates.forEach((u) => u(props ?? {})));
  host.flush();
};

/** Команда `invoke` из ядра: команда компонента на всех его экземплярах. */
(globalThis as any).__keyfill_invoke = (json: string) => {
  const { target, command, params } = JSON.parse(json) as { target: string; command: string; params: Record<string, unknown> };
  for (const i of instancesOf(target, 'invoke')) i.invoke(command, params);
  host.flush();
};

/** Монтирует сцену или сценарий. Пропсы — из CLI (`--props`, `--set`) или из случая теста. */
export function mount(entry: Props, input: PropsInput = (globalThis as any).__keyfill_props ?? {}) {
  const scene = entry.$cues ? entry.scene : entry;
  const { accessors, errors } = sceneProps(scene, input);
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
  if (!isElement(root)) throw new KeyfillError('scene.root должен вернуть элемент');
  host.insert(stage, root.id);
  if (scene.timelines?.length) host.emit({ op: 'timelines', items: scene.timelines });
  if (entry.$cues) host.emit({ op: 'cues', items: entry.items });
  host.flush();
}

// ─── Канал ──────────────────────────────────────────────────────────────────

export const defineChannel: typeof S.defineChannel = (def) => brand({ ...def, $channel: true });

type CatalogItem = ((p: Props) => KeyfillElement) & { keyfillName: string; keyfillSchema: Record<string, PropDesc> };

/**
 * Состояние канала на стороне JS: каталог и живые экземпляры. Слоями, порядком и движением
 * ведает ядро; сюда приходят только монтирование, данные, команды и снятие.
 */
const channel = {
  stage: 0,
  catalog: new Map<string, CatalogItem>(),
  live: new Map<string, { inst: Instance; created: Instance[]; dispose: () => void }>(),
};

function mountChannel(def: Props) {
  const stage = host.create('stage');
  channel.stage = stage;
  host.emit({ op: 'stage', id: stage, ref: def.stage.ref, safe: def.stage.safe, fps: def.fps, fonts: def.fonts ?? [], systemFonts: def.systemFonts ?? true });
  for (const c of def.components ?? []) {
    const name = (c as Partial<CatalogItem> | undefined)?.keyfillName;
    if (typeof name !== 'string') throw new KeyfillError('defineChannel: в components — компоненты из component()');
    if (channel.catalog.has(name)) throw new KeyfillError(`defineChannel: ${name} в components дважды`);
    channel.catalog.set(name, c as CatalogItem);
  }
  host.emit({ op: 'channel', components: [...channel.catalog.keys()] });
  if (def.timelines?.length) host.emit({ op: 'timelines', items: def.timelines });
  host.flush();
}

type LayerCmd =
  | { op: 'mount'; component: string; props?: Props; before?: number }
  | { op: 'update'; inst: string; props: Props }
  | { op: 'invoke'; inst: string; command: string; params?: Props }
  | { op: 'release'; inst: string }
  | { op: 'unmount'; inst: string };

const reject = (errors: string[]) => host.emit({ op: 'layer', reply: 'rejected', errors });
const values = (i: Instance) => host.emit({ op: 'layer', reply: 'values', inst: i.name, props: i.current() });

function liveInstance(name: string): Instance {
  const item = channel.live.get(name);
  if (!item) throw new KeyfillError(`канал: экземпляра ${name} нет`);
  return item.inst;
}

function layerCmd(c: LayerCmd) {
  switch (c.op) {
    case 'mount': {
      const C = channel.catalog.get(c.component);
      if (!C) {
        return reject([`в каталоге канала нет компонента "${c.component}"; есть: ${[...channel.catalog.keys()].join(', ') || 'ничего'}`]);
      }
      const errors: string[] = [];
      fields(C.keyfillSchema, c.props ?? {}, '', errors, true, {}, c.component);
      if (errors.length) return reject(errors);
      // Исключение в view: отбрасываем недостроенное дерево, экземпляр в каталог живых не попадает.
      const mark = host.mark();
      const count = instances.length;
      let mounted: { value: KeyfillElement; dispose: () => void };
      try {
        mounted = createRoot(() => C({ ...(c.props ?? {}) }));
      } catch (e) {
        host.rollback(mark);
        instances = instances.slice(0, count);
        throw e;
      }
      // Корень экземпляра и вложенные компоненты: снимаются вместе.
      const created = instances.slice(count);
      const inst = created.find((i) => i.root === mounted.value.id)!;
      host.insert(channel.stage, mounted.value.id, c.before);
      channel.live.set(inst.name, { inst, created, dispose: mounted.dispose });
      host.emit({ op: 'layer', reply: 'mounted', inst: inst.name, root: inst.root });
      return values(inst);
    }
    case 'update': {
      const inst = liveInstance(c.inst);
      const errors = inst.checkUpdate(c.props);
      if (errors.length) return reject(errors);
      inst.update(c.props);
      return values(inst);
    }
    case 'invoke': {
      const inst = liveInstance(c.inst);
      const errors = inst.checkInvoke(c.command, c.params ?? {});
      if (errors.length) return reject(errors);
      inst.invoke(c.command, c.params ?? {});
      return values(inst);
    }
    // release — уйти через out (узлы удалит ядро после ухода), unmount — убрать сразу.
    case 'release':
    case 'unmount': {
      const item = channel.live.get(c.inst);
      if (!item) return;
      item.dispose();
      host.emit({ op: c.op === 'release' ? 'release' : 'remove', id: item.inst.root });
      channel.live.delete(c.inst);
      instances = instances.filter((i) => !item.created.includes(i));
      return;
    }
  }
}

/** Команда канала из ядра. Пакеты уходят и при исключении: ядро применит то, что успело измениться. */
(globalThis as any).__keyfill_channel = (json: string) => {
  const c = JSON.parse(json) as LayerCmd;
  try {
    layerCmd(c);
  } catch (e) {
    // Команда упала посреди исполнения: ядру нужны пропсы экземпляра после частичного эффекта.
    const item = 'inst' in c ? channel.live.get(c.inst) : undefined;
    if (item && c.op !== 'unmount' && c.op !== 'release') values(item.inst);
    throw e;
  } finally {
    host.flush();
  }
};

// ─── Тесты и конфиг ─────────────────────────────────────────────────────────

type TestItem = { name: string; scene: Props; props?: Props; [k: string]: unknown };
const tests: TestItem[] = [];

export const test: typeof S.test = (name, c) => {
  if (typeof name !== 'string' || !name.trim()) throw new KeyfillError('test: нужно имя случая');
  const scene = (c as Props | undefined)?.scene as Props | undefined;
  if (!scene || !(scene.$cues || scene.root)) throw new KeyfillError(`test "${name}": scene — defineScene(...) или cues(scene, ...)`);
  if (tests.some((x) => x.name === name)) throw new KeyfillError(`test: два случая с именем "${name}"`);
  tests.push({ ...(c as Props), name, scene });
};

export const defineConfig: typeof S.defineConfig = (c) => brand({ ...c, $config: true });

/**
 * Точка входа бандла. Что делать с файлом, говорит ядро (`__keyfill_entry`): `'tests'` — отдать
 * список случаев теста, номер — смонтировать сцену случая с его пропсами; иначе файл — сцена,
 * сценарий или keyfill.config.ts, по тому, что он экспортирует.
 */
export function __keyfill_main(value: unknown) {
  const entry = (globalThis as any).__keyfill_entry as 'tests' | number | null;
  if (entry === 'tests') {
    host.emit({ op: 'tests', items: tests.map(({ scene: _s, props: _p, ...rest }) => rest) });
    host.flush();
    return;
  }
  if (typeof entry === 'number') {
    const c = tests[entry];
    if (!c) throw new KeyfillError(`в файле нет случая №${entry}`);
    mount(c.scene, { values: c.props ?? {} });
    return;
  }
  const v = value as Props | undefined;
  if (v?.$channel) return mountChannel(v);
  if (v?.$config) {
    const { $config: _, ...config } = v;
    host.emit({ op: 'config', value: config });
    host.flush();
    return;
  }
  if (!v && tests.length) throw new KeyfillError('это файл тестов: запускай keyfill test <файл>');
  if (!v || !(v.$cues || v.root)) {
    throw new KeyfillError('файл не экспортирует сцену: export default defineScene({ ... }), cues(scene, ...) или defineChannel({ ... })');
  }
  mount(v);
}

const notYet = (name: string) => (): never => {
  throw new KeyfillError(`source.${name} появится в M6`);
};

export const source: typeof S.source = { json: notYet('json'), ws: notYet('ws'), osc: notYet('osc') };
