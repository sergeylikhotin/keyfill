import { defineTheme, hex } from '@keyfill/core';

export const theme = defineTheme({
  color: {
    surface: hex('#101418', 0.92),
    surfaceLive: hex('#2A0F16', 0.92),
    accent: hex('#FF3355'),
    text: hex('#FFFFFF'),
    muted: hex('#A9B4C2'),
  },
  font: {
    display: { family: 'Inter' },
  },
});
