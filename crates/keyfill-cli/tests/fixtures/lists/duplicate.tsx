// Повторяющийся ключ: первый элемент показан, остальные пропущены с предупреждением.
import { Box, For, Text, defineScene } from '@keyfill/core';

export default defineScene({
  stage: { ref: [640, 360] },
  fps: 25,
  root: () => (
    <Box>
      <For each={['a', 'b', 'a']}>{(s) => <Text>{s}</Text>}</For>
    </Box>
  ),
});
