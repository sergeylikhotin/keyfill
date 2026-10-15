// Сверка реализации со спецификацией: по строке на каждый экспорт spec/kine.d.ts.
// tsc падает, если экспорта нет или сигнатура реализации несовместима. В бандл файл не попадает.
// Сгенерировано из spec/kine.d.ts; при добавлении экспорта в спеку добавь строку сюда.
import type * as Spec from '@kine/core';
import * as Impl from './index';

export const signal: typeof Spec.signal = Impl.signal;
export const computed: typeof Spec.computed = Impl.computed;
export const effect: typeof Spec.effect = Impl.effect;
export const batch: typeof Spec.batch = Impl.batch;
export const untrack: typeof Spec.untrack = Impl.untrack;
export const pct: typeof Spec.pct = Impl.pct;
export const hex: typeof Spec.hex = Impl.hex;
export const oklch: typeof Spec.oklch = Impl.oklch;
export const on: typeof Spec.on = Impl.on;
export const row: typeof Spec.row = Impl.row;
export const column: typeof Spec.column = Impl.column;
export const grid: typeof Spec.grid = Impl.grid;
export const stack: typeof Spec.stack = Impl.stack;
export const tween: typeof Spec.tween = Impl.tween;
export const spring: typeof Spec.spring = Impl.spring;
export const animate: typeof Spec.animate = Impl.animate;
export const fade: typeof Spec.fade = Impl.fade;
export const slide: typeof Spec.slide = Impl.slide;
export const seq: typeof Spec.seq = Impl.seq;
export const par: typeof Spec.par = Impl.par;
export const delay: typeof Spec.delay = Impl.delay;
export const pulse: typeof Spec.pulse = Impl.pulse;
export const prop: typeof Spec.prop = Impl.prop;
export const component: typeof Spec.component = Impl.component;
export const timeline: typeof Spec.timeline = Impl.timeline;
export const Box: typeof Spec.Box = Impl.Box;
export const Text: typeof Spec.Text = Impl.Text;
export const Span: typeof Spec.Span = Impl.Span;
export const Image: typeof Spec.Image = Impl.Image;
export const Shape: typeof Spec.Shape = Impl.Shape;
export const defineTheme: typeof Spec.defineTheme = Impl.defineTheme;
export const setTheme: typeof Spec.setTheme = Impl.setTheme;
export const defineScene: typeof Spec.defineScene = Impl.defineScene;
export const cues: typeof Spec.cues = Impl.cues;
export const source: typeof Spec.source = Impl.source;
