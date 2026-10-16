// Две плашки в одном месте на разных слоях: вход второй во время ухода первой — наложение.
import { Box, component, defineChannel, fade, hex, sequence, tween } from '@keyfill/core';

const card = (name: string, color: `#${string}`) =>
  component({
    name,
    props: {},
    presence: { in: fade([0, 1], tween(0.4)), out: fade([1, 0], tween(0.4)) },
    view: () => <Box anchor={{ to: 'stage', at: 'center' }} size={{ w: 200, h: 100 }} look={{ background: hex(color) }} />,
  });

const A = card('A', '#cc3333');
const B = card('B', '#3333cc');

export default defineChannel({
  stage: { ref: [640, 360] },
  fps: 25,
  layers: { a: 10, b: 20 },
  states: () => ({ one: { a: A.with({}) }, two: { b: B.with({}) } }),
  // one → two — всё сразу (наложение), two → one — по очереди.
  transitions: { 'two -> one': sequence() },
});
