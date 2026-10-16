// Сцена с исключением в своём коде: стек должен указывать сюда, а не в бандл.
import { Box, defineScene } from '@keyfill/core';

function width(): number {
  const missing: { w: number } | undefined = undefined;
  return missing!.w;
}

export default defineScene({
  stage: { ref: [320, 180] },
  fps: 25,
  root: () => <Box size={[width(), 10]} />,
});
