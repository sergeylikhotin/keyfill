import { Box, Text, component, fade, on, par, prop, pulse, row, column, slide, spring, stagger, tween } from '@kine/core';
import { theme } from './theme';

export const LowerThird = component({
  name: 'LowerThird',
  props: { name: String, role: String, live: Boolean, tags: prop<string[]>() },
  parts: ['panel', 'dot', 'title', 'subtitle'],

  view: ({ name, role, live }, part) => (
    <Box
      name={part.panel}
      anchor={{ to: 'titleSafe', at: 'bottom-left' }}
      layout={row({ gap: 16, align: 'center' })}
      look={[
        { background: theme.color.surface, padding: [16, 24], radius: 12, transition: { background: tween(0.4) } },
        on(live, { background: theme.color.surfaceLive, border: { width: 2, paint: theme.color.accent } }),
      ]}
      must={{ inside: 'titleSafe' }}
    >
      <Box
        name={part.dot}
        when={live}
        size={{ w: 16, h: 16 }}
        look={{ background: theme.color.accent, radius: 8 }}
        presence={{ loop: pulse({ period: 1.2, opacity: [1, 0.4] }), out: fade([1, 0], tween(0.2)) }}
      />
      {/* Имя и должность появляются по очереди: каскад на детях колонки. */}
      <Box layout={column({ gap: 4 })} presence={{ in: stagger(0.08, par(fade([0, 1], tween(0.25)), slide({ y: [10, 0] }, tween(0.3, 'out')))) }}>
        <Text
          name={part.title}
          look={{ font: theme.font.display, size: 56, weight: 600, color: theme.color.text }}
          fit={{ shrinkTo: 32, then: 'ellipsis' }}
          must={{ fit: true, contrast: 4.5 }}
          else={[{ shrinkTo: 28 }, { shrinkTo: 28, lines: 2 }, { shrinkTo: 28, lines: 2, ellipsis: true }]}
        >
          {name}
        </Text>
        <Text name={part.subtitle} look={{ font: theme.font.display, size: 28, color: theme.color.muted, case: 'upper', tracking: 0.08 }}>
          {role}
        </Text>
      </Box>
    </Box>
  ),

  presence: {
    in: [slide({ x: [-40, 0] }, spring.snappy), fade([0, 1], tween(0.2))],
    out: fade([1, 0], tween(0.25)),
  },
});
