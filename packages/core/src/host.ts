// Мост в нативное ядро: команды копятся в очереди и уходят одним вызовом на кадр.

export type Cmd =
  | { op: 'create'; id: number; kind: string; src?: string }
  | { op: 'set'; id: number; key: string; value: unknown }
  | { op: 'insert'; parent: number; child: number }
  | { op: 'text'; id: number; runs: { text: string; look?: unknown; src?: string; dynamic?: unknown }[] }
  | { op: 'theme'; values: Record<string, unknown> }
  | {
      op: 'stage';
      id: number;
      ref: [number, number];
      safe?: { action?: number; title?: number };
      fps: number;
      fonts?: string[];
      systemFonts?: boolean;
    }
  | { op: 'diag'; level: 'info' | 'warn' | 'error'; rule: string; node: string; source?: string; message: string; hint?: string }
  | { op: 'cues'; items: { t: number; cmd: string; target?: string; props?: Record<string, unknown>; command?: string }[] }
  | { op: 'timelines'; items: readonly unknown[] }
  /** play / next / stop / emit от команды компонента; ядро исполнит в момент команды сценария. */
  | { op: 'motion'; inst: string; cmd: string; marker?: string; name?: string; data?: unknown }
  /** Схема пропсов сцены и ошибки входных данных из CLI. */
  | { op: 'schema'; props: PropInfo[]; errors: string[] }
  /** Описание компонента для `kine schema` и пульта. */
  | { op: 'component'; name: string; props: PropInfo[]; parts: string[]; commands: { name: string; params: PropInfo[] }[] };

export type PropInfo = { name: string; type: string; default?: unknown; required?: boolean };

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
  runs(id: number, runs: { text: string; look?: unknown; src?: string; dynamic?: unknown }[]) {
    queue.push({ op: 'text', id, runs });
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
