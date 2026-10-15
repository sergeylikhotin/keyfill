// Черновик публичного API @kine/core (v0). Это спецификация, не реализация:
// типы фиксируют контракт, который агент читает и который проверяет tsc.

declare module '@kine/core' {
  // Только явные export видны снаружи; бренды ниже — внутренние.
  export {};

  // ─── Сигналы ──────────────────────────────────────────────────────────────

  export type Accessor<T> = () => T;
  export type Signal<T> = Accessor<T> & { set(v: T | ((prev: T) => T)): void };
  /** Значение или реактивный аксессор. Функция в пропсе = подписка. */
  export type Reactive<T> = T | Accessor<T>;

  export function signal<T>(initial: T): Signal<T>;
  export function computed<T>(fn: () => T): Accessor<T>;
  export function effect(fn: () => void | (() => void)): void;
  export function batch(fn: () => void): void;
  /** Явное чтение без подписки; глушит предупреждение signal.untracked. */
  export function untrack<T>(fn: () => T): T;

  // ─── Время ────────────────────────────────────────────────────────────────

  // Времени в JS нет: движение исполняет ядро (presence, transition, timeline), а JS работает
  // только на командах сценария и смене данных. Поэтому кадр вычислим в любой момент t.

  // ─── Единицы и значения ───────────────────────────────────────────────────

  /** Логические единицы опорного кадра (stage.ref). */
  export type Len = number;
  export type Pct = { readonly pct: number };
  export function pct(v: number): Pct;
  export type Size = Len | Pct | 'auto' | 'content';
  export type Edges = Len | readonly [y: Len, x: Len] | readonly [top: Len, right: Len, bottom: Len, left: Len];
  export type Corners = Len | readonly [tl: Len, tr: Len, br: Len, bl: Len];
  export type Vec2 = readonly [x: Len, y: Len];

  const tokenBrand: unique symbol;
  /** Ссылка на токен темы. Разрешается в ядре, поэтому смена темы не перезапускает JS. */
  export type Token<T> = { readonly [tokenBrand]: T };

  export type Color =
    | { readonly r: number; readonly g: number; readonly b: number; readonly a?: number }
    | { readonly oklch: readonly [l: number, c: number, h: number]; readonly a?: number }
    | Token<'color'>;
  export function hex(v: `#${string}`, alpha?: number): Color;
  export function oklch(l: number, c: number, h: number, alpha?: number): Color;

  export type GradientStop = readonly [offset: number, color: Color];
  export type Paint =
    | Color
    | { readonly linear: { readonly angle: number; readonly stops: readonly GradientStop[] } }
    | { readonly radial: { readonly center?: Vec2; readonly radius?: Len; readonly stops: readonly GradientStop[] } };

  const fontFileBrand: unique symbol;
  /** Файл шрифта, полученный импортом: `import inter from './Inter-Regular.otf'`. */
  export type FontFile = string & { readonly [fontFileBrand]: true };

  /** Семейство по имени: из файлов сцены (defineScene.fonts) или системное. */
  export type FontRef = Token<'font'> | { readonly family: string };

  // ─── look: стиль узла ─────────────────────────────────────────────────────

  export interface Shadow {
    readonly x?: Len;
    readonly y?: Len;
    readonly blur?: Len;
    readonly spread?: Len;
    readonly color: Color;
    readonly inset?: boolean;
  }

  export interface Transform {
    readonly x?: Len;
    readonly y?: Len;
    readonly scale?: number | readonly [number, number];
    /** Градусы. */
    readonly rotate?: number;
    /** Доли собственного размера, [0.5, 0.5] = центр. */
    readonly origin?: readonly [number, number];
  }

  export type BlendMode = 'normal' | 'multiply' | 'screen' | 'overlay' | 'add';

  export interface BoxLook {
    readonly background?: Paint;
    readonly padding?: Edges;
    readonly radius?: Corners;
    readonly border?: { readonly width: Len; readonly paint: Paint; readonly align?: 'inside' | 'center' | 'outside' };
    readonly shadow?: Shadow | readonly Shadow[];
    readonly opacity?: number;
    readonly blend?: BlendMode;
    readonly clip?: boolean;
    readonly transform?: Transform;
    readonly transition?: Transitions;
  }

  export interface TextLook extends BoxLook {
    readonly font?: FontRef;
    readonly size?: Len;
    readonly weight?: 100 | 200 | 300 | 400 | 500 | 600 | 700 | 800 | 900;
    readonly color?: Paint;
    readonly lineHeight?: number;
    /** Межбуквенный интервал в em. */
    readonly tracking?: number;
    readonly align?: 'start' | 'center' | 'end';
    readonly case?: 'none' | 'upper' | 'lower';
  }

  /** Условный слой. Реактивен, в отличие от `cond() && layer`, который вычисляется один раз. */
  const onBrand: unique symbol;
  export type OnLayer<L> = { readonly [onBrand]: L };
  export function on<L>(cond: Accessor<boolean>, layer: L): OnLayer<L>;

  /** Слои применяются по порядку, побеждает последний. Ложные значения пропускаются. */
  export type LookInput<L> = L | OnLayer<L> | false | null | undefined | readonly LookInput<L>[];

  // ─── Раскладка ────────────────────────────────────────────────────────────

  export type Align = 'start' | 'center' | 'end' | 'stretch' | 'baseline';
  export type Justify = 'start' | 'center' | 'end' | 'between' | 'around' | 'evenly';
  export type Track = Len | Pct | 'auto' | { readonly fr: number };

  export type Layout =
    | { readonly kind: 'row' | 'column'; readonly gap?: Len; readonly align?: Align; readonly justify?: Justify; readonly wrap?: boolean }
    | { readonly kind: 'grid'; readonly columns: readonly Track[]; readonly rows?: readonly Track[]; readonly gap?: Len | Vec2 }
    /** Дети накладываются друг на друга, каждый позиционируется своим anchor. */
    | { readonly kind: 'stack' };

  type FlexOpts = { gap?: Len; align?: Align; justify?: Justify; wrap?: boolean };
  export function row(o?: FlexOpts): Layout;
  export function column(o?: FlexOpts): Layout;
  export function grid(o: { columns: readonly Track[]; rows?: readonly Track[]; gap?: Len | Vec2 }): Layout;
  export function stack(): Layout;

  export type AnchorPoint =
    | 'top-left' | 'top' | 'top-right'
    | 'left' | 'center' | 'right'
    | 'bottom-left' | 'bottom' | 'bottom-right';

  export type AnchorTarget = 'stage' | 'actionSafe' | 'titleSafe' | 'parent' | PartRef;

  export interface Anchor {
    /** По умолчанию `parent`. */
    readonly to?: AnchorTarget;
    /** Точка цели. */
    readonly at: AnchorPoint;
    /** Точка самого узла, по умолчанию совпадает с `at`. */
    readonly self?: AnchorPoint;
    readonly offset?: Vec2;
  }

  // ─── fit, must, else ──────────────────────────────────────────────────────

  export interface Fit {
    /** Минимальный кегль, до которого можно ужимать текст. */
    readonly shrinkTo?: Len;
    readonly lines?: number;
    readonly then?: 'ellipsis' | 'clip';
  }

  export interface Must {
    /** Контент не переполняет узел. */
    readonly fit?: true;
    readonly inside?: AnchorTarget;
    /** Минимальный WCAG-контраст текста к фактическому фону под ним. */
    readonly contrast?: number;
    readonly visible?: true;
  }

  /**
   * Вариант деградации. Варианты независимые, как ViewThatFits: else[k] применяется сам по себе,
   * без шагов перед ним (эскалацию пиши явно: [{ shrinkTo: 28 }, { shrinkTo: 28, lines: 2 }]).
   * В dev нарушение must — ошибка с подсказкой, какой вариант её исправит; в эфире применяется
   * первый подходящий, а если не подошёл ни один — последний.
   */
  export interface ElseStep<L> {
    readonly shrinkTo?: Len;
    readonly lines?: number;
    readonly ellipsis?: true;
    readonly look?: LookInput<L>;
    readonly hide?: true;
  }

  // ─── Движение ─────────────────────────────────────────────────────────────

  export type EaseCurve = 'linear' | 'in' | 'out' | 'inOut' | readonly [x1: number, y1: number, x2: number, y2: number];
  export interface Tween { readonly kind: 'tween'; readonly duration: number; readonly ease: EaseCurve }
  export interface Spring { readonly kind: 'spring'; readonly stiffness: number; readonly damping: number; readonly mass: number }
  export type Easing = Tween | Spring;

  export function tween(duration: number, ease?: EaseCurve): Tween;
  export const spring: ((o: { stiffness: number; damping: number; mass?: number }) => Spring) & {
    readonly snappy: Spring;
    readonly gentle: Spring;
    readonly bouncy: Spring;
  };

  /** Свойства, которые можно анимировать. */
  export interface Animatable {
    readonly x: Len;
    readonly y: Len;
    readonly scale: number;
    readonly rotate: number;
    readonly opacity: number;
    readonly width: Len;
    readonly height: Len;
    readonly radius: Len;
    readonly blur: Len;
    readonly background: Color;
    readonly color: Color;
    /** Доля видимых символов текста, 0..1 (эффект печати). */
    readonly reveal: number;
  }
  export type AnimKey = keyof Animatable;
  /** Целевое значение или пара [from, to]. */
  export type AnimTargets = { readonly [K in AnimKey]?: Animatable[K] | readonly [Animatable[K], Animatable[K]] };

  export type Transitions = { readonly [K in AnimKey]?: Easing } & { readonly all?: Easing };

  const motionBrand: unique symbol;
  export type Motion = { readonly [motionBrand]: true; readonly duration: number };

  export function animate(targets: AnimTargets, easing?: Easing): Motion;
  export function fade(range: readonly [number, number], easing?: Easing): Motion;
  export function slide(o: { x?: readonly [Len, Len]; y?: readonly [Len, Len] }, easing?: Easing): Motion;
  export function seq(...items: Motion[]): Motion;
  export function par(...items: Motion[]): Motion;
  export function delay(seconds: number, m: Motion): Motion;
  /** Бесконечный цикл, обычно для presence.loop. */
  export function pulse(o: { period: number; opacity?: readonly [number, number]; scale?: readonly [number, number] }): Motion;

  export interface Presence {
    readonly in?: Motion | readonly Motion[];
    readonly loop?: Motion | readonly Motion[];
    readonly out?: Motion | readonly Motion[];
  }

  // ─── Части и компоненты ───────────────────────────────────────────────────

  const partBrand: unique symbol;
  /** Именованная часть компонента. Таймлайны адресуют узлы только через части. */
  export type PartRef<N extends string = string> = { readonly [partBrand]: N };
  export type Parts<N extends string> = { readonly [K in N]: PartRef<K> };

  const propBrand: unique symbol;
  export type PropType<T> = { readonly [propBrand]: T };
  /** Тип пропса, который не выражается конструктором: prop<Player[]>(). */
  export function prop<T>(): PropType<T>;

  export type PropSchema = Record<string, StringConstructor | NumberConstructor | BooleanConstructor | PropType<unknown>>;
  export type PropValue<S> =
    S extends StringConstructor ? string :
    S extends NumberConstructor ? number :
    S extends BooleanConstructor ? boolean :
    S extends PropType<infer T> ? T : never;
  export type InferProps<S extends PropSchema> = { readonly [K in keyof S]: PropValue<S[K]> };
  /** Внутри компонента каждый пропс — аксессор. */
  export type PropAccessors<P> = { readonly [K in keyof P]: Accessor<P[K]> };
  /** Снаружи можно передать значение или аксессор. */
  export type ReactiveProps<P> = { readonly [K in keyof P]: Reactive<P[K]> };

  /** То, что команда компонента может сделать с экземпляром. */
  export interface CommandContext<P> {
    /** Текущие пропсы экземпляра. */
    readonly props: PropAccessors<P>;
    /** Новые значения своих пропсов, как команда сценария `update`. */
    set(patch: Partial<P>): void;
    /** Сыграть сегмент таймлайна от маркера до следующего и вернуться туда, где был; без маркера — play. */
    play(marker?: string): void;
    next(): void;
    stop(): void;
    /** Своё событие для пульта и журнала. */
    emit(name: string, data?: unknown): void;
  }

  export interface CommandDef<P, S extends PropSchema> {
    readonly params: S;
    readonly run: (params: InferProps<S>, ctx: CommandContext<P>) => void;
  }

  export interface ComponentDef<S extends PropSchema, N extends string, C extends Record<string, PropSchema>> {
    /** Имя для дампа и ошибок: экземпляры называются `LowerThird#1`. */
    readonly name: string;
    readonly props: S;
    readonly parts?: readonly N[];
    readonly view: (props: PropAccessors<InferProps<S>>, part: Parts<N>) => KineElement;
    /** Появление и уход корня. Если у компонента есть таймлайн в сцене, управляет он, а не presence. */
    readonly presence?: Presence;
    /** Действия, которые пульт показывает как кнопки с параметрами: `at(t, 'invoke', Scoreboard, 'goal', { side: 'home' })`. */
    readonly commands?: { readonly [K in keyof C]: CommandDef<InferProps<S>, C[K]> };
  }

  const componentBrand: unique symbol;
  export interface Component<P, N extends string, C = {}> {
    (props: ReactiveProps<P> & NodePlacement): KineElement;
    readonly [componentBrand]: { readonly props: P; readonly parts: N; readonly commands: C };
  }
  export type PartsOf<C> = C extends Component<any, infer N, any> ? N : never;
  export type PropsOf<C> = C extends Component<infer P, any, any> ? P : never;
  /** Команды компонента и типы их параметров. */
  export type CommandsOf<C> = C extends Component<any, any, infer K> ? K : never;

  export function component<
    const S extends PropSchema,
    const N extends string = never,
    const C extends Record<string, PropSchema> = {},
  >(def: ComponentDef<S, N, C>): Component<InferProps<S>, N, { readonly [K in keyof C]: InferProps<C[K]> }>;

  // ─── Таймлайн (*.motion.ts) ───────────────────────────────────────────────

  const keyBrand: unique symbol;
  export type Key<N extends string> = { readonly [keyBrand]: N };

  const emitBrand: unique symbol;
  export type TimelineEmit = { readonly [emitBrand]: true };

  export interface KeyBuilder<N extends string> {
    /** В момент t анимировать часть `part`. Имя части проверяется по компоненту. */
    at(t: number, part: N, targets: AnimTargets, easing?: Easing): Key<N>;
    /** Своё событие, когда воспроизведение проходит момент t. */
    emit(t: number, name: string): TimelineEmit;
  }

  export interface TimelineLoop<M extends string> {
    readonly from: M;
    readonly to: M;
    /** repeat — с начала, pingpong — туда и обратно. По умолчанию repeat. */
    readonly mode?: 'repeat' | 'pingpong';
    /** Сколько повторов; без него — пока не придёт next или stop. */
    readonly count?: number;
  }

  /**
   * Воспроизведение:
   * - play — от 0 вперёд; на маркере из `holds` стоп до next; отрезок `loop` повторяется до next;
   *   обычное воспроизведение не заходит за маркер `out`;
   * - stop — прыжок на `out` и воспроизведение до следующего маркера (или конца), затем экземпляр снят;
   * - next — снимает текущую остановку: продолжить после hold или выйти из цикла;
   * - ctx.play('goal') в команде — сегмент от маркера до следующего, затем возврат туда, где был.
   * Прыжки (stop, сегмент) продолжают каналы с текущих значений; каналы без ключей после прыжка доигрывают своё.
   * Без явного `loop`, если есть маркеры `loop` и `out`, повторяется отрезок loop..out.
   */
  export interface TimelineDef<N extends string, M extends string> {
    readonly keys: readonly Key<N>[];
    readonly markers?: { readonly [K in M]: number };
    readonly loop?: TimelineLoop<NoInfer<M>>;
    readonly holds?: readonly NoInfer<M>[];
    readonly events?: readonly TimelineEmit[];
  }

  const timelineBrand: unique symbol;
  export type Timeline<C> = { readonly [timelineBrand]: C };

  export function timeline<C extends Component<any, any, any>, const M extends string = never>(
    target: C,
    build: (k: KeyBuilder<PartsOf<C>>) => TimelineDef<PartsOf<C>, M>,
  ): Timeline<C>;

  // ─── Примитивы ────────────────────────────────────────────────────────────

  export type KineElement = { readonly __kine: 'element' };
  export type Child = KineElement | string | number | Accessor<string | number> | false | null | undefined | readonly Child[];

  /** То, как узел размещается в родителе. Есть у примитивов и у компонентов. */
  export interface NodePlacement {
    readonly anchor?: Anchor;
    readonly size?: { readonly w?: Size; readonly h?: Size };
    readonly min?: { readonly w?: Size; readonly h?: Size };
    readonly max?: { readonly w?: Size; readonly h?: Size };
    readonly grow?: number;
    readonly shrink?: number;
    readonly aspect?: number;
    /** Ячейка в родительском grid, 1-based. Без неё — авторазмещение по порядку. */
    readonly cell?: { readonly col?: number; readonly row?: number; readonly colSpan?: number; readonly rowSpan?: number };
    /** Монтирование с учётом presence: при false узел уходит через out. */
    readonly when?: Reactive<boolean>;
  }

  export interface NodeProps<L> extends NodePlacement {
    readonly name?: PartRef;
    readonly look?: Reactive<LookInput<L>>;
    readonly presence?: Presence;
    readonly must?: Must;
    readonly else?: readonly ElseStep<L>[];
  }

  export function Box(p: NodeProps<BoxLook> & { layout?: Layout; children?: Child }): KineElement;
  export function Text(p: NodeProps<TextLook> & { fit?: Fit; children?: Child }): KineElement;
  /** Только внутри Text, наследует его текстовые свойства. */
  export function Span(p: { look?: Reactive<LookInput<TextLook>>; children?: Child }): KineElement;
  export function Image(p: NodeProps<BoxLook> & { src: string; fit?: 'cover' | 'contain' | 'fill' }): KineElement;
  export function Shape(p: NodeProps<BoxLook> & { path: string; stroke?: { width: Len; paint: Paint }; fill?: Paint }): KineElement;

  // ─── Тема, сцена, данные ──────────────────────────────────────────────────

  /** Листья темы превращаются в токены: значения подставляет ядро. */
  export type Tokens<T> = {
    readonly [K in keyof T]: T[K] extends Color ? Token<'color'> : T[K] extends FontRef ? Token<'font'> : Tokens<T[K]>;
  };
  export function defineTheme<const T>(values: T): Tokens<T> & { readonly __values: T };
  export function setTheme<T>(theme: Tokens<T>, values: T): void;

  export interface StageConfig {
    readonly ref: Vec2;
    /** Доли кадра, по умолчанию action 0.93, title 0.90 (EBU R95). */
    readonly safe?: { readonly action?: number; readonly title?: number };
  }

  export interface SceneDef {
    readonly stage: StageConfig;
    readonly fps: number;
    readonly root: () => KineElement;
    /** Файлы шрифтов из импортов; путь разрешает бандлер. Семейства берутся из самих файлов. */
    readonly fonts?: readonly FontFile[];
    /** Системные шрифты; для детерминированного рендера выключай и подключай файлы. По умолчанию true. */
    readonly systemFonts?: boolean;
    /** Таймлайны компонентов из *.motion.ts: у каждого экземпляра компонента — своё воспроизведение. */
    readonly timelines?: readonly Timeline<any>[];
  }
  export function defineScene(def: SceneDef): SceneDef;

  // ─── Сценарий (*.cues.ts) ─────────────────────────────────────────────────

  const cueBrand: unique symbol;
  export type Cue = { readonly [cueBrand]: true };

  export interface CueBuilder {
    /**
     * play — узлы с presence начинают in, таймлайны — воспроизведение; stop — out, сцена снимается
     * после последнего out; next — снять остановку таймлайна (hold или цикл). С компонентом —
     * только его экземпляры.
     */
    at(t: number, cmd: 'play' | 'stop' | 'next', target?: Component<any, any, any>): Cue;
    /** Новые значения пропсов всех экземпляров компонента; изменения анимируются через transition. */
    at<C extends Component<any, any, any>>(t: number, cmd: 'update', target: C, props: Partial<PropsOf<C>>): Cue;
    /** Команда компонента с параметрами, на всех его экземплярах. */
    at<C extends Component<any, any, any>, K extends keyof CommandsOf<C> & string>(
      t: number,
      cmd: 'invoke',
      target: C,
      command: K,
      params: CommandsOf<C>[K],
    ): Cue;
  }

  const cuesBrand: unique symbol;
  export type Cues = { readonly [cuesBrand]: true };

  /**
   * Сценарий команд для рендера в файл и проверок: `kine render scene.cues.ts --t 2.4`.
   * Команда применяется на границе кадра (время округляется вверх до кадра).
   * Без сценария сцена получает play в момент 0.
   */
  export function cues(scene: SceneDef, build: (b: CueBuilder) => readonly Cue[]): Cues;

  export const source: {
    json<T>(path: string, o?: { watch?: boolean }): Accessor<T | undefined>;
    ws<T>(url: string): Accessor<T | undefined>;
    osc(port: number, address: string): Accessor<number | string | undefined>;
  };
}

declare module '@kine/core/jsx-runtime' {
  import type { KineElement } from '@kine/core';
  export namespace JSX {
    type Element = KineElement;
    // Строчных интринсиков нет: все примитивы — импортируемые компоненты.
    interface IntrinsicElements {}
    interface ElementChildrenAttribute { children: {} }
  }
  export function jsx(type: unknown, props: unknown, key?: unknown): KineElement;
  export function jsxs(type: unknown, props: unknown, key?: unknown): KineElement;
  export const Fragment: unique symbol;
}

declare module '@kine/core/jsx-dev-runtime' {
  export * from '@kine/core/jsx-runtime';
  export function jsxDEV(type: unknown, props: unknown, key: unknown, isStatic: boolean, source: { fileName: string; lineNumber: number; columnNumber: number }, self: unknown): import('@kine/core').KineElement;
}

// Импорт шрифта даёт FontFile: бандлер kine подставляет абсолютный путь к файлу.
declare module '*.otf' {
  const font: import('@kine/core').FontFile;
  export default font;
}
declare module '*.ttf' {
  const font: import('@kine/core').FontFile;
  export default font;
}
declare module '*.woff' {
  const font: import('@kine/core').FontFile;
  export default font;
}
declare module '*.woff2' {
  const font: import('@kine/core').FontFile;
  export default font;
}
