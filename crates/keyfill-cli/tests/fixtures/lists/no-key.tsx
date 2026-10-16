// Объекты без by: ключа нет — ошибка с местом в TSX (tsc ловит это раньше, здесь — рантайм).
import { Box, For, Text, defineScene } from '@keyfill/core';

const rows = [{ name: 'a' }, { name: 'b' }];

export default defineScene({
  stage: { ref: [640, 360] },
  fps: 25,
  root: () => (
    <Box>
      {/* @ts-expect-error by обязателен для объектов */}
      <For each={rows}>{(r) => <Text>{() => r().name}</Text>}</For>
    </Box>
  ),
});
