// Синтаксическая ошибка: сборка должна вернуть build.syntax с местом в этом файле.
import { Box, defineScene } from '@keyfill/core';

export default defineScene({
  stage: { ref: [320, 180] },
  fps: 25,
  root: () => <Box /> ;
});
