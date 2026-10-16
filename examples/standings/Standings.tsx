import { Box, For, Text, component, count, fade, grid, par, prop, slide, spring, stagger, tween } from '@keyfill/core';
import { theme } from '../scoreboard/theme';

/** Строка таблицы: данные приходят списком, каждая строка — элемент For по id команды. */
const team = prop.object({
  id: prop.string({ description: 'Код команды: ключ строки' }),
  name: String,
  points: prop.number({ min: 0, int: true }),
});

/** Список команд по местам; сцена берёт тот же тип для своих данных. */
export const teams = prop.list(team, { max: 8 });

const cell = { font: theme.font.ui, size: 30, color: theme.color.text } as const;

export const Standings = component({
  name: 'Standings',
  props: {
    title: prop.string({ default: 'Турнирная таблица' }),
    teams,
  },
  parts: ['panel', 'title'],

  view: ({ title, teams }, part) => (
    <Box
      name={part.panel}
      anchor={{ to: 'titleSafe', at: 'top-right' }}
      size={{ w: 560 }}
      look={{ background: theme.color.panel, padding: [20, 24], radius: 14, transition: { layout: spring.gentle } }}
      must={{ inside: 'titleSafe' }}
    >
      <Text name={part.title} look={{ font: theme.font.ui, size: 22, color: theme.color.muted, case: 'upper', tracking: 0.08 }}>
        {title}
      </Text>
      {/* Новые строки въезжают каскадом, ушедшие уходят тем же каскадом наоборот. */}
      <Box
        layout={grid({ columns: [{ fr: 1 }], gap: [0, 6] })}
        presence={{
          in: stagger(0.06, par(fade([0, 1], tween(0.25)), slide({ x: [24, 0] }, tween(0.3, 'out')))),
          out: stagger(0.04, fade([1, 0], tween(0.2))),
        }}
      >
        <For each={teams} by={(t) => t.id}>
          {(t, i) => (
            // Строка едет на новое место: перестановка, уход соседа выше.
            <Box layout={grid({ columns: [40, { fr: 1 }, 64], gap: [12, 0] })} look={{ padding: [6, 0], transition: { layout: spring.snappy } }}>
              <Text look={{ ...cell, color: theme.color.muted }}>{() => String(i() + 1)}</Text>
              <Text look={cell} fit={{ shrinkTo: 22, then: 'ellipsis' }} must={{ fit: true }}>{() => t().name}</Text>
              <Text look={{ ...cell, weight: 600, align: 'end' }}>{count(() => t().points)}</Text>
            </Box>
          )}
        </For>
      </Box>
    </Box>
  ),
});
