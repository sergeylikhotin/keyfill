import { defineTheme, hex } from '@keyfill/core';

export const theme = defineTheme({
  color: {
    panel: hex('#0E1116'),
    divider: hex('#2A313C'),
    text: hex('#F4F6F8'),
    muted: hex('#9AA6B5'),
    live: hex('#FF4D5E'),
    accent: hex('#3DD6A5'),
    onAccent: hex('#06231A'),
  },
  font: {
    ui: { family: 'Inter' },
  },
});
