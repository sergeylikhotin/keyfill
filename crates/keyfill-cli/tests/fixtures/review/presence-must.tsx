import { Box, Text, animate, defineScene, tween } from '@keyfill/core';
export default defineScene({
  stage: { ref: [400, 200] },
  fps: 10,
  root: () => (
    <Box size={{ w: 400, h: 200 }}>
      <Box size={{ w: 300, h: 60 }} look={{ background: { r: 0, g: 0, b: 0 } }}
        presence={{ in: animate({ width: [20, 300] }, tween(1)) }}>
        <Text look={{ size: 30 }} must={{ fit: true }}>{'Hello'}</Text>
      </Box>
    </Box>
  ),
});
