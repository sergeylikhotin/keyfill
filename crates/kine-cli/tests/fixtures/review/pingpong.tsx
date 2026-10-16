import { Box, component, defineScene, timeline, tween } from '@kine/core';
export const Card = component({
  name: 'Card', props: {}, parts: ['body', 'dot'],
  commands: { flash: { params: {}, run: (_p, ctx) => ctx.play('flash') } },
  view: (_p, part) => (
    <Box name={part.body} size={{ w: 50, h: 50 }} look={{ background: { r: 255, g: 0, b: 0 } }}>
      <Box name={part.dot} size={{ w: 10, h: 10 }} look={{ background: { r: 0, g: 0, b: 255 } }} />
    </Box>
  ),
});
const motion = timeline(Card, ({ at }) => ({
  keys: [at(1, 'body', { opacity: [1, 0] }, tween(1, 'linear')), at(3, 'dot', { x: [0, 20] }, tween(0.5, 'linear'))],
  markers: { loop: 1, back: 2, out: 2.5, flash: 3, end: 3.5 },
  loop: { from: 'loop', to: 'back', mode: 'pingpong' },
}));
export default defineScene({ stage: { ref: [400, 200] }, fps: 10, timelines: [motion], root: () => <Box><Card /></Box> });
