// Реализация @kine/core для спайка M0. Публичный контракт — spec/kine.d.ts;
// здесь типы намеренно рыхлые, сверка реализации со спецификацией — в M1.

import { host } from './host';
import { batch, computed, effect, signal, untrack, type Accessor } from './signals';

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
const read = <T>(v: T | Accessor<T>): T => (typeof v === 'function' ? (v as Accessor<T>)() : v);

function flat(v: unknown, out: unknown[] = []): unknown[] {
  if (Array.isArray(v)) for (const x of v) flat(x, out);
  else out.push(v);
  return out;
}

// ─── Время (в M0 всегда кадр 0) ─────────────────────────────────────────────

export const time = signal(0);
export const frame = signal(0);

// ─── Значения ───────────────────────────────────────────────────────────────

export const pct = (v: number) => ({ pct: v });

export function hex(v: string, alpha = 1) {
  const m = /^#([0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/i.exec(v);
  if (!m) throw new KineError(`hex(): ожидается #RGB, #RRGGBB или #RRGGBBAA, получено "${v}"`);
  let h = m[1];
  if (h.length === 3) h = [...h].map((c) => c + c).join('');
  const n = (i: number) => parseInt(h.slice(i, i + 2), 16);
  const a = h.length === 8 ? (n(6) / 255) * alpha : alpha;
  return { r: n(0), g: n(2), b: n(4), a };
}

export const oklch = (l: number, c: number, h: number, alpha = 1) => ({ oklch: [l, c, h], a: alpha });

export const on = (cond: Accessor<boolean>, layer: unknown) => ({ $on: cond, layer });

// ─── Раскладка ──────────────────────────────────────────────────────────────

export const row = (o: Props = {}) => ({ kind: 'row', ...o });
export const column = (o: Props = {}) => ({ kind: 'column', ...o });
export const grid = (o: Props) => ({ kind: 'grid', ...o });
export const stack = () => ({ kind: 'stack' });

// ─── Движение: в M0 только данные, ядро их не исполняет ─────────────────────

export const tween = (duration: number, ease: unknown = 'inOut') => ({ kind: 'tween', duration, ease });
const springOf = (o: { stiffness: number; damping: number; mass?: number }) => ({ kind: 'spring', mass: 1, ...o });
export const spring = Object.assign(springOf, {
  snappy: springOf({ stiffness: 400, damping: 30 }),
  gentle: springOf({ stiffness: 120, damping: 20 }),
  bouncy: springOf({ stiffness: 300, damping: 12 }),
});

export const animate = (targets: Props, easing: unknown = tween(0.3)) => ({ motion: 'animate', targets, easing });
export const fade = (range: [number, number], easing?: unknown) => animate({ opacity: range }, easing);
export const slide = (o: Props, easing?: unknown) => animate(o, easing);
export const seq = (...items: unknown[]) => ({ motion: 'seq', items });
export const par = (...items: unknown[]) => ({ motion: 'par', items });
export const delay = (seconds: number, m: unknown) => ({ motion: 'delay', seconds, item: m });
export const pulse = (o: Props) => ({ motion: 'pulse', ...o });

export function timeline(target: { kineName?: string }, build: (k: Props) => Props) {
  const at = (t: number, part: string, targets: Props, easing?: unknown) => ({ t, part, targets, easing });
  return { timeline: target.kineName, ...build({ at }) };
}

// ─── Тема ───────────────────────────────────────────────────────────────────

const isLeaf = (v: any) =>
  v === null || typeof v !== 'object' || 'r' in v || 'oklch' in v || 'family' in v || Array.isArray(v);

export function defineTheme<T extends Props>(values: T) {
  const flatValues: Record<string, unknown> = {};
  const walk = (v: any, path: string[]): any => {
    if (isLeaf(v)) {
      const p = path.join('.');
      flatValues[p] = v;
      return { $t: p };
    }
    const out: Props = {};
    for (const k of Object.keys(v)) out[k] = walk(v[k], [...path, k]);
    return out;
  };
  const tokens = walk(values, []);
  host.emit({ op: 'theme', values: flatValues });
  return Object.assign(tokens, { __values: values });
}

export function setTheme(_theme: unknown, values: Props) {
  const flatValues: Record<string, unknown> = {};
  const walk = (v: any, path: string[]) => {
    if (isLeaf(v)) flatValues[path.join('.')] = v;
    else for (const k of Object.keys(v)) walk(v[k], [...path, k]);
  };
  walk(values, []);
  host.emit({ op: 'theme', values: flatValues });
}

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

const PLACEMENT = ['anchor', 'size', 'min', 'max', 'grow', 'shrink', 'aspect'] as const;
const STATIC = ['layout', 'fit', 'must', 'presence', 'src', 'path', 'stroke', 'fill'] as const;

function applyPlacement(id: number, p: Props) {
  for (const k of PLACEMENT) if (p[k] !== undefined) host.set(id, k, p[k]);
  if (p.when !== undefined) effect(() => host.set(id, 'hidden', !read(p.when)));
}

function createNode(kind: string, p: Props): number {
  const id = host.create(kind, p.__src);
  applyPlacement(id, p);
  for (const k of STATIC) if (p[k] !== undefined) host.set(id, k, p[k]);
  if (p.else !== undefined) host.set(id, 'else', p.else);
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
    if (c === null || c === undefined || c === false || c === true) continue;
    if (!isElement(c)) {
      throw new KineError(`Box принимает только элементы; текст оборачивай в <Text>. Получено: ${JSON.stringify(c)}`, p.__src);
    }
    host.insert(id, c.id);
  }
  return element(id);
}

export function Text(p: Props): KineElement {
  const id = createNode('text', p);
  const parts = flat(p.children).filter((c) => c !== null && c !== undefined && c !== false && c !== true);
  for (const c of parts) {
    if (isElement(c)) throw new KineError('Span внутри Text появится в M1', p.__src);
  }
  effect(() => host.text(id, parts.map((c) => String(read(c as string))).join('')));
  return element(id);
}

export function Image(p: Props): KineElement {
  return element(createNode('image', p));
}

export function Shape(p: Props): KineElement {
  return element(createNode('shape', p));
}

export function Span(_p: Props): KineElement {
  throw new KineError('Span появится в M1');
}

// ─── Компоненты ─────────────────────────────────────────────────────────────

export const prop = () => ({ $prop: true });

const counters = new Map<string, number>();

export function component(def: Props) {
  if (typeof def.name !== 'string') throw new KineError('component(): нужно поле name');
  const C = (p: Props): KineElement => {
    const n = (counters.get(def.name) ?? 0) + 1;
    counters.set(def.name, n);
    const inst = `${def.name}#${n}`;

    const accessors: Props = {};
    for (const k of Object.keys(def.props ?? {})) {
      if (!(k in p)) throw new KineError(`${def.name}: не передан пропс "${k}"`, p.__src);
      const v = p[k];
      accessors[k] = typeof v === 'function' ? v : () => v;
    }
    const parts: Props = {};
    for (const name of def.parts ?? []) parts[name] = { $part: name, inst };

    const root = untrack(() => def.view(accessors, parts));
    if (!isElement(root)) throw new KineError(`${def.name}.view должен вернуть один элемент`, p.__src);
    host.set(root.id, 'instance', { name: inst, src: p.__src });
    if (def.presence) host.set(root.id, 'presence', def.presence);
    applyPlacement(root.id, p);
    return root;
  };
  return Object.assign(C, { kineName: def.name });
}

// ─── Сцена ──────────────────────────────────────────────────────────────────

export const defineScene = (def: Props) => def;

export function mount(scene: Props) {
  const stage = host.create('stage');
  host.emit({ op: 'stage', id: stage, ref: scene.stage.ref, safe: scene.stage.safe, fps: scene.fps });
  const root = untrack(() => scene.root());
  if (!isElement(root)) throw new KineError('scene.root должен вернуть элемент');
  host.insert(stage, root.id);
  host.flush();
}

export const source = new Proxy(
  {},
  {
    get(_, key) {
      return () => {
        throw new KineError(`source.${String(key)} появится в M3`);
      };
    },
  },
);
