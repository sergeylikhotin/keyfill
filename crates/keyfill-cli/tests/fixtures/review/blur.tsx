import { Box, component, defineScene, timeline, tween } from '@keyfill/core';
const Card = component({
  name: 'Card', props: {}, parts: ['body'],
  view: (_p, part) => <Box name={part.body} size={{ w: 50, h: 50 }} look={{ background: { r: 255, g: 0, b: 0 } }} />,
});
const motion = timeline(Card, ({ at }) => ({
  keys: [at(0, 'body', { opacity: [0, 1] }, tween(0.5)), at(1, 'body', { opacity: 0, blur: 4 }, tween(0.5))],
  markers: { out: 1 },
}));
export default defineScene({ stage: { ref: [400, 200] }, fps: 10, timelines: [motion], root: () => <Box><Card /></Box> });
