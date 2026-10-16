import { Box, Span, Text, animate, clock, component, count, fade, grid, spring, stack, tween } from '@keyfill/core';
import { theme } from './theme';

const team = { font: theme.font.ui, size: 34, weight: 600, color: theme.color.text } as const;
const score = { font: theme.font.ui, size: 40, weight: 600, color: theme.color.text, align: 'end' } as const;

export const Scoreboard = component({
  name: 'Scoreboard',
  props: {
    home: String,
    away: String,
    homeScore: Number,
    awayScore: Number,
    period: String,
    /** Время матча в секундах; идёт само, пока clockRunning. */
    clockFrom: Number,
    clockRunning: Boolean,
    live: Boolean,
  },
  parts: ['panel', 'home', 'away', 'homeScore', 'awayScore', 'status', 'badge'],

  view: ({ home, away, homeScore, awayScore, period, clockFrom, clockRunning, live }, part) => (
    <Box name={part.panel} anchor={{ to: 'titleSafe', at: 'top-left' }} layout={stack()} must={{ inside: 'titleSafe' }}>
      <Box
        layout={grid({ columns: [{ fr: 1 }, 72], gap: [16, 10] })}
        size={{ w: 520 }}
        look={{ background: theme.color.panel, padding: [20, 24], radius: 14 }}
      >
        <Text
          name={part.home}
          look={team}
          fit={{ shrinkTo: 26 }}
          must={{ fit: true, contrast: 4.5 }}
          else={[{ lines: 2 }, { lines: 2, ellipsis: true }]}
        >
          {home}
        </Text>
        <Text name={part.homeScore} look={score}>{count(homeScore)}</Text>
        <Text
          name={part.away}
          look={team}
          fit={{ shrinkTo: 26 }}
          must={{ fit: true, contrast: 4.5 }}
          else={[{ lines: 2 }, { lines: 2, ellipsis: true }]}
        >
          {away}
        </Text>
        <Text name={part.awayScore} look={score}>{count(awayScore)}</Text>
        <Text name={part.status} cell={{ colSpan: 2 }} look={{ font: theme.font.ui, size: 22, color: theme.color.muted }}>
          <Span look={{ color: theme.color.live, weight: 600 }}>● LIVE</Span>
          {' · '}
          {period}
          {' · '}
          {clock({ from: clockFrom, running: clockRunning, format: 'mm:ss' })}
        </Text>
        {/* Подчёркивание привязано к части status: второй проход раскладки ставит его под текст. */}
        <Box
          anchor={{ to: part.status, at: 'bottom-left', self: 'top-left', offset: [0, 6] }}
          size={{ w: 48, h: 3 }}
          look={{ background: theme.color.live, radius: 2 }}
        />
      </Box>
      <Box
        name={part.badge}
        when={live}
        anchor={{ at: 'top-right', offset: [14, -14] }}
        look={{ background: theme.color.accent, padding: [6, 12], radius: 10 }}
        presence={{ in: [fade([0, 1], tween(0.15)), animate({ scale: [0.6, 1] }, spring.bouncy)], out: fade([1, 0], tween(0.2)) }}
      >
        <Text look={{ font: theme.font.ui, size: 18, weight: 600, color: theme.color.onAccent, tracking: 0.06 }}>4K HDR</Text>
      </Box>
    </Box>
  ),

  commands: {
    // Гол: счёт +1 и вспышка цифры — сегмент таймлайна goalHome или goalAway.
    goal: {
      params: { side: String },
      run: ({ side }, ctx) => {
        if (side === 'home') ctx.set({ homeScore: ctx.props.homeScore() + 1 });
        else ctx.set({ awayScore: ctx.props.awayScore() + 1 });
        ctx.play(side === 'home' ? 'goalHome' : 'goalAway');
        ctx.emit('goal');
      },
    },
  },
});
