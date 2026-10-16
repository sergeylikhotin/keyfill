import { Box, Text, column, component, fade, on, slide, spring, tween } from '@kine/core';
import { theme } from './theme';

// Плашка у нижнего края titleSafe: имя и должность. Части (parts) — адреса узлов в дампе
// (__Name__#1/title) и цели для таймлайна.
export const __Name__ = component({
  name: '__Name__',
  props: { name: String, role: String, live: Boolean },
  parts: ['panel', 'title', 'subtitle'],

  view: ({ name, role, live }, part) => (
    <Box
      name={part.panel}
      anchor={{ to: 'titleSafe', at: 'bottom-left' }}
      layout={column({ gap: 4 })}
      look={[
        { background: theme.color.surface, padding: [16, 24], radius: 12, transition: { background: tween(0.4) } },
        // Реактивный слой: пересчитывается, когда меняется live.
        on(live, { background: theme.color.surfaceLive }),
      ]}
      must={{ inside: 'titleSafe' }}
    >
      <Text
        name={part.title}
        look={{ font: theme.font.display, size: 56, weight: 600, color: theme.color.text }}
        fit={{ shrinkTo: 32, then: 'ellipsis' }}
        must={{ fit: true, contrast: 4.5 }}
      >
        {name}
      </Text>
      <Text name={part.subtitle} look={{ font: theme.font.display, size: 28, color: theme.color.muted, case: 'upper' }}>
        {role}
      </Text>
    </Box>
  ),

  presence: {
    in: [slide({ x: [-40, 0] }, spring.snappy), fade([0, 1], tween(0.2))],
    out: fade([1, 0], tween(0.25)),
  },
});
