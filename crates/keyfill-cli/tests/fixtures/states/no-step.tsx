// Шаг, которого в переходе нет: в one → two слой a уходит, а не входит.
import { Box, component, defineChannel } from '@keyfill/core';

const A = component({ name: 'A', props: {}, view: () => <Box /> });
const B = component({ name: 'B', props: {}, view: () => <Box /> });

export default defineChannel({
  stage: { ref: [640, 360] },
  fps: 25,
  layers: { a: 10, b: 20 },
  states: () => ({ one: { a: A.with({}) }, two: { b: B.with({}) } }),
  transitions: {
    'one -> two': ({ enter }) => [enter('a', { after: 'b:in' })],
  },
});
