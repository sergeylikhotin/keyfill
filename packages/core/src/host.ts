// Мост в нативное ядро: команды копятся в очереди и уходят одним вызовом на кадр.

export type Cmd =
  | { op: 'create'; id: number; kind: string; src?: string }
  | { op: 'set'; id: number; key: string; value: unknown }
  | { op: 'insert'; parent: number; child: number }
  | { op: 'text'; id: number; value: string }
  | { op: 'theme'; values: Record<string, unknown> }
  | { op: 'stage'; id: number; ref: [number, number]; safe?: { action?: number; title?: number }; fps: number };

declare const __kine_emit: (json: string) => void;

let queue: Cmd[] = [];
let nextId = 1;

export const host = {
  create(kind: string, src?: string): number {
    const id = nextId++;
    queue.push(src ? { op: 'create', id, kind, src } : { op: 'create', id, kind });
    return id;
  },
  set(id: number, key: string, value: unknown) {
    queue.push({ op: 'set', id, key, value });
  },
  insert(parent: number, child: number) {
    queue.push({ op: 'insert', parent, child });
  },
  text(id: number, value: string) {
    queue.push({ op: 'text', id, value });
  },
  emit(cmd: Cmd) {
    queue.push(cmd);
  },
  flush() {
    if (!queue.length) return;
    const q = queue;
    queue = [];
    __kine_emit(JSON.stringify(q));
  },
};
