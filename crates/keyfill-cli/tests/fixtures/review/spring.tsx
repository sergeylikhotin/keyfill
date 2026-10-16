import { Box, defineScene, slide, spring } from '@keyfill/core';
export default defineScene({
  stage: { ref: [400, 200] },
  fps: 100,
  root: () => (
    <Box size={{ w: 400, h: 200 }}>
      <Box size={{ w: 50, h: 50 }} look={{ background: { r: 255, g: 0, b: 0 } }}
        presence={{ in: slide({ x: [-100, 0] }, spring({ stiffness: 10, damping: 200 })) }} />
      <Box size={{ w: 50, h: 50 }} look={{ background: { r: 0, g: 255, b: 0 } }}
        presence={{ in: slide({ x: [-100, 0] }, spring({ stiffness: 100, damping: 0.5 })) }} />
    </Box>
  ),
});
