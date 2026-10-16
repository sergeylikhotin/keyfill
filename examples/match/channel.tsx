// Канал матча с состояниями: оператор переключает не плашки, а состояние сцены, и меняет данные.
//   keyfill schema examples/match/channel.tsx        — состояния, слои, данные
//   keyfill filmstrip examples/match/show.journal.jsonl --t 0..9 --n 12
import { defineChannel, prop, sequence } from '@keyfill/core';
import interRegular from '../../assets/fonts/inter/Inter-Regular.otf';
import interSemiBold from '../../assets/fonts/inter/Inter-SemiBold.otf';
import { LowerThird } from '../lower-third/LowerThird';
import { Scoreboard } from '../scoreboard/Scoreboard';
import scoreboardMotion from '../scoreboard/Scoreboard.motion';

export default defineChannel({
  stage: { ref: [1920, 1080] },
  fps: 50,
  fonts: [interRegular, interSemiBold],
  systemFonts: false,
  timelines: [scoreboardMotion],
  // Данные матча: их видят все плашки; меняет их update без слоя.
  props: {
    home: prop.string({ default: 'Динамо Москва' }),
    away: prop.string({ default: 'Крылья Советов Самара' }),
    homeScore: prop.number({ min: 0, int: true, default: 0 }),
    awayScore: prop.number({ min: 0, int: true, default: 0 }),
    period: prop.string({ default: '1-й тайм' }),
    clockFrom: prop.number({ min: 0, default: 0 }),
    guest: prop.string({ default: '' }),
    guestRole: prop.string({ default: '' }),
  },
  layers: { score: 10, lower: 20 },
  states: (d) => ({
    // До матча: только представление встречи.
    pregame: {
      lower: LowerThird.with({ name: () => `${d.home()} — ${d.away()}`, role: 'Скоро начало', live: false }),
    },
    // Игра: табло со счётом и часами.
    live: {
      score: Scoreboard.with({ home: d.home, away: d.away, homeScore: d.homeScore, awayScore: d.awayScore, period: d.period, clockFrom: d.clockFrom, live: true }),
    },
    // Интервью по ходу игры: табло остаётся (тот же экземпляр), снизу — гость.
    interview: {
      score: Scoreboard.with({ home: d.home, away: d.away, homeScore: d.homeScore, awayScore: d.awayScore, period: d.period, clockFrom: d.clockFrom, live: true }),
      lower: LowerThird.with({ name: d.guest, role: d.guestRole, live: true }),
    },
    // Перерыв: часы стоят, табло не в режиме live.
    break: {
      score: Scoreboard.with({ home: d.home, away: d.away, homeScore: d.homeScore, awayScore: d.awayScore, period: 'Перерыв', clockFrom: d.clockFrom, clockRunning: false, live: false }),
    },
  }),
  // Когда начинается каждый шаг перехода; остальные пары — всё сразу (together).
  transitions: {
    // Представление встречи уходит, табло входит через 0.1 с после.
    'pregame -> live': sequence(0.1),
    // Табло переходит на новые пропсы сразу, гость появляется через 0.3 с после этого.
    'live -> interview': ({ enter }) => [enter('lower', { with: 'score:update', offset: 0.3 })],
  },
});
