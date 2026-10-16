import { Box, defineScene, fade, par, slide, tween } from '@kine/core';
export default defineScene({
  stage: { ref: [400, 200] },
  fps: 10,
  props: { vis: Boolean },
  defaults: { vis: true },
  root: (p) => (
    <Box size={{ w: 400, h: 200 }}>
      <Box when={p.vis} size={{ w: 50, h: 50 }} look={{ background: { r: 255, g: 0, b: 0 } }}
        presence={{ in: fade([0, 1], tween(0.2)), out: par(fade([1, 0], tween(0.3)), slide({ y: [0, 40] }, tween(0.3))) }} />
    </Box>
  ),
});
