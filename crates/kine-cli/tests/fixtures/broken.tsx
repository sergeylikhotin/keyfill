// Синтаксическая ошибка: сборка должна вернуть build.syntax с местом в этом файле.
import { Box, defineScene } from '@kine/core';

export default defineScene({
  stage: { ref: [320, 180] },
  fps: 25,
  root: () => <Box /> ;
});
