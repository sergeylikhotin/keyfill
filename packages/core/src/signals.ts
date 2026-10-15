// Минимальные сигналы в духе Solid: синхронные, с автоматическим трекингом зависимостей.

type Computation = { run(): void; deps: Set<Set<Computation>>; disposed: boolean };

let current: Computation | null = null;
let batchDepth = 0;
const pending = new Set<Computation>();

export type Accessor<T> = () => T;
export type Signal<T> = Accessor<T> & { set(v: T | ((prev: T) => T)): void };

function track(subs: Set<Computation>) {
  if (current) {
    subs.add(current);
    current.deps.add(subs);
  }
}

function notify(subs: Set<Computation>) {
  for (const c of [...subs]) pending.add(c);
  if (batchDepth === 0) drain();
}

function drain() {
  while (pending.size) {
    const c = pending.values().next().value!;
    pending.delete(c);
    if (!c.disposed) c.run();
  }
}

function cleanup(c: Computation) {
  for (const subs of c.deps) subs.delete(c);
  c.deps.clear();
}

export function signal<T>(initial: T): Signal<T> {
  let value = initial;
  const subs = new Set<Computation>();
  const read = (() => {
    track(subs);
    return value;
  }) as Signal<T>;
  read.set = (v) => {
    const next = typeof v === 'function' ? (v as (p: T) => T)(value) : v;
    if (Object.is(next, value)) return;
    value = next;
    notify(subs);
  };
  return read;
}

export function effect(fn: () => void | (() => void)): void {
  let dispose: void | (() => void);
  const c: Computation = {
    deps: new Set(),
    disposed: false,
    run() {
      if (typeof dispose === 'function') dispose();
      cleanup(c);
      const prev = current;
      current = c;
      try {
        dispose = fn();
      } finally {
        current = prev;
      }
    },
  };
  c.run();
}

export function computed<T>(fn: () => T): Accessor<T> {
  const s = signal<T>(undefined as T);
  effect(() => s.set(() => fn()));
  return s;
}

export function batch(fn: () => void): void {
  batchDepth++;
  try {
    fn();
  } finally {
    batchDepth--;
    if (batchDepth === 0) drain();
  }
}

export function untrack<T>(fn: () => T): T {
  const prev = current;
  current = null;
  try {
    return fn();
  } finally {
    current = prev;
  }
}
