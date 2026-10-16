// Минимальные сигналы в духе Solid: синхронные, с автоматическим трекингом зависимостей.

type Computation = { run(): void; deps: Set<Set<Computation>>; disposed?: boolean; dispose?(): void };

let current: Computation | null = null;
// Владелец эффектов: экземпляр компонента на канале. Эффекты, созданные под ним (и в их
// перезапусках), снимаются вместе с экземпляром.
let owner: Computation[] | null = null;
// Пока исполняется тело view, чтение сигнала вне реактивного контекста — почти наверняка ошибка:
// значение прочитается один раз и больше не обновится.
let viewGuard: { report: () => void } | null = null;
let batchDepth = 0;
const pending = new Set<Computation>();

export type Accessor<T> = () => T;
export type Signal<T> = Accessor<T> & { set(v: T | ((prev: T) => T)): void };

function track(subs: Set<Computation>) {
  if (current) {
    subs.add(current);
    current.deps.add(subs);
  } else if (viewGuard) {
    viewGuard.report();
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
    c.run();
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
  const own = owner;
  const c: Computation = {
    deps: new Set(),
    run() {
      if (c.disposed) return;
      if (typeof dispose === 'function') dispose();
      cleanup(c);
      const prev = current;
      const prevOwner = owner;
      current = c;
      owner = own;
      try {
        dispose = fn();
      } finally {
        current = prev;
        owner = prevOwner;
      }
    },
    dispose() {
      c.disposed = true;
      if (typeof dispose === 'function') dispose();
      cleanup(c);
    },
  };
  own?.push(c);
  c.run();
}

/**
 * Исполняет `fn` владельцем эффектов; `dispose` снимает все эффекты, созданные под ним.
 * Корень внутри другого владельца (элемент списка в экземпляре) снимается и вместе с ним.
 */
export function createRoot<T>(fn: () => T): { value: T; dispose: () => void } {
  const list: Computation[] = [];
  const parent = owner;
  const dispose = () => {
    list.splice(0).forEach((c) => c.dispose!());
    const i = parent?.indexOf(link) ?? -1;
    if (i >= 0) parent!.splice(i, 1);
  };
  const link: Computation = { deps: new Set(), run() {}, dispose };
  parent?.push(link);
  owner = list;
  try {
    return { value: fn(), dispose };
  } finally {
    owner = parent;
  }
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
  const guard = viewGuard;
  current = null;
  viewGuard = null;
  try {
    return fn();
  } finally {
    current = prev;
    viewGuard = guard;
  }
}

/** Исполняет тело view: без трекинга, но с сигналом о чтении сигналов вне аксессоров. */
export function runView<T>(onUntrackedRead: () => void, fn: () => T): T {
  const prev = current;
  const guard = viewGuard;
  let reported = false;
  current = null;
  viewGuard = {
    report() {
      if (!reported) {
        reported = true;
        onUntrackedRead();
      }
    },
  };
  try {
    return fn();
  } finally {
    current = prev;
    viewGuard = guard;
  }
}
