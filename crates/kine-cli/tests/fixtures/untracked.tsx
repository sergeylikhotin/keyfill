// Типичная ошибка агента: live() прочитан прямо в теле view, значение не будет обновляться.
import { Box, component, defineScene, signal } from '@kine/core';

const Dot = component({
  name: 'Dot',
  props: { live: Boolean },
  view: ({ live }) => <Box size={{ w: 10, h: 10 }} look={live() ? { opacity: 1 } : { opacity: 0.2 }} />,
});

export default defineScene({
  stage: { ref: [320, 180] },
  fps: 25,
  systemFonts: false,
  root: () => <Dot live={signal(true)} />,
});
