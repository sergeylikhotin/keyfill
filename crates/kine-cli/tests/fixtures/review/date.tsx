import { Box, Text, defineScene } from '@kine/core';
const d = new Date();
export default defineScene({
  stage: { ref: [400, 200] },
  fps: 10,
  root: () => <Box size={{ w: 400, h: 200 }}><Text>{d.toISOString() + ' ' + new Date(0).getHours() + ' ' + Date()}</Text></Box>,
});
