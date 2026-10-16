import { Box, Text, defineScene, hex } from '@kine/core';
import interRegular from '__FONTS__/Inter-Regular.otf';

export default defineScene({
  stage: { ref: [1920, 1080] },
  fps: 50,
  fonts: [interRegular],
  systemFonts: false,
  props: { title: String },
  defaults: { title: 'kine' },
  root: (p) => (
    <Box anchor={{ to: 'titleSafe', at: 'center' }} look={{ background: hex('#101418', 0.92), padding: [16, 32], radius: 12 }}>
      <Text look={{ font: { family: 'Inter' }, size: 64, color: hex('#FFFFFF') }} must={{ fit: true }}>
        {p.title}
      </Text>
    </Box>
  ),
});
