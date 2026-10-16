// Карточка матча: одна подложка, два облика. В заставке (`intro`) — названия команд по центру
// кадра, в игре (`score`) — счёт и часы в углу. Канал меняет облик пропсом `mode` и место
// якорем состояния, а экземпляр остаётся тем же: подложка едет и меняет размер сама
// (`transition.layout`), содержимое сменяется своим `in`. Морфинг — это композиция, а не механизм.
import { Box, Text, clock, column, component, count, delay, fade, grid, prop, slide, spring, stack, tween } from '@keyfill/core';
import { theme } from '../scoreboard/theme';

const team = { font: theme.font.ui, size: 34, weight: 600, color: theme.color.text } as const;
const score = { font: theme.font.ui, size: 40, weight: 600, color: theme.color.text, align: 'end' } as const;
// Новое содержимое проявляется, когда подложка уже в пути.
const appear = { in: delay(0.15, fade([0, 1], tween(0.25))) };

export const MatchCard = component({
  name: 'MatchCard',
  props: {
    mode: prop.enum(['intro', 'score'], { default: 'score' }),
    home: String,
    away: String,
    homeScore: prop.number({ min: 0, int: true, default: 0 }),
    awayScore: prop.number({ min: 0, int: true, default: 0 }),
    period: prop.string({ default: '' }),
    clockFrom: prop.number({ min: 0, default: 0 }),
    clockRunning: prop.boolean({ default: false }),
    live: prop.boolean({ default: false }),
    note: prop.string({ default: 'Скоро начало' }),
  },
  parts: ['panel'],
  presence: { in: [fade([0, 1], tween(0.3)), slide({ y: [-24, 0] }, spring.snappy)], out: fade([1, 0], tween(0.3)) },
  view: (p, part) => (
    <Box
      name={part.panel}
      anchor={{ to: 'titleSafe', at: 'top-left' }}
      layout={stack()}
      look={{ background: theme.color.panel, padding: [20, 24], radius: 16, clip: true, transition: { layout: spring.gentle } }}
    >
      <Box when={() => p.mode() === 'intro'} size={{ w: 900 }} layout={column({ gap: 10, align: 'center' })} presence={appear}>
        <Text look={{ font: theme.font.ui, size: 52, weight: 600, color: theme.color.text }} fit={{ shrinkTo: 36 }}>
          {() => `${p.home()} — ${p.away()}`}
        </Text>
        <Text look={{ font: theme.font.ui, size: 26, color: theme.color.muted, case: 'upper', tracking: 0.08 }}>{p.note}</Text>
      </Box>
      <Box when={() => p.mode() === 'score'} size={{ w: 472 }} layout={grid({ columns: [{ fr: 1 }, 72], gap: [16, 10] })} presence={appear}>
        <Text look={team} fit={{ shrinkTo: 26 }}>{p.home}</Text>
        <Text look={score}>{count(p.homeScore)}</Text>
        <Text look={team} fit={{ shrinkTo: 26 }}>{p.away}</Text>
        <Text look={score}>{count(p.awayScore)}</Text>
        <Text cell={{ colSpan: 2 }} look={{ font: theme.font.ui, size: 22, color: theme.color.muted }}>
          {() => (p.live() ? '● LIVE · ' : '') + p.period()}
          {' · '}
          {clock({ from: p.clockFrom, running: p.clockRunning, format: 'mm:ss' })}
        </Text>
      </Box>
    </Box>
  ),
  commands: {
    // Гол — это данные канала (update homeScore); команда только сообщает пульту.
    goal: { params: { side: prop.enum(['home', 'away']) }, run: ({ side }, ctx) => ctx.emit('goal', { side }) },
  },
});

// Короткая плашка под карточкой: ставится якорем к её подложке в состоянии канала.
export const Tag = component({
  name: 'Tag',
  props: { text: String },
  presence: { in: [fade([0, 1], tween(0.2)), slide({ y: [-12, 0] }, spring.snappy)], out: fade([1, 0], tween(0.2)) },
  view: ({ text }) => (
    <Box look={{ background: theme.color.live, padding: [8, 16], radius: 10 }}>
      <Text look={{ font: theme.font.ui, size: 24, weight: 600, color: theme.color.text, case: 'upper', tracking: 0.06 }}>{text}</Text>
    </Box>
  ),
});
