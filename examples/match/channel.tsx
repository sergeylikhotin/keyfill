// Канал матча с состояниями: оператор переключает не плашки, а состояние сцены, и меняет данные.
//   keyfill schema examples/match/channel.tsx        — состояния, слои, данные
//   keyfill filmstrip examples/match/show.journal.jsonl --t 0..11 --n 12
import { defineChannel, prop, sequence } from '@keyfill/core';
import interRegular from '../../assets/fonts/inter/Inter-Regular.otf';
import interSemiBold from '../../assets/fonts/inter/Inter-SemiBold.otf';
import { LowerThird } from '../lower-third/LowerThird';
import { MatchCard, Tag } from './MatchCard';

export default defineChannel({
  stage: { ref: [1920, 1080] },
  fps: 50,
  fonts: [interRegular, interSemiBold],
  systemFonts: false,
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
  layers: { score: 10, lower: 20, note: 30 },
  states: (d) => {
    // Карточка матча на слое score — один экземпляр во всех состояниях, где она есть.
    const card = (extra: { mode?: 'intro' | 'score'; period?: string; clockRunning?: boolean; live?: boolean }) =>
      MatchCard.with({ home: d.home, away: d.away, homeScore: d.homeScore, awayScore: d.awayScore, period: d.period, clockFrom: d.clockFrom, ...extra });
    return {
      // Заставка: карточка встречи по центру кадра. В live та же карточка уезжает в угол и
      // становится табло: другой облик (mode) и другое место (якорь состояния).
      intro: { score: MatchCard.with({ home: d.home, away: d.away, mode: 'intro' }, { anchor: { to: 'stage', at: 'center' } }) },
      // До матча: только представление встречи снизу.
      pregame: { lower: LowerThird.with({ name: () => `${d.home()} — ${d.away()}`, role: 'Скоро начало', live: false }) },
      // Игра: счёт и часы.
      live: { score: card({ clockRunning: true, live: true }) },
      // Интервью по ходу игры: карточка остаётся (тот же экземпляр), снизу — гость.
      interview: { score: card({ clockRunning: true, live: true }), lower: LowerThird.with({ name: d.guest, role: d.guestRole, live: true }) },
      // Проверка VAR: под карточкой — плашка, привязанная к её подложке (часть panel слоя score).
      var: {
        score: card({ clockRunning: true, live: true }),
        note: Tag.with({ text: 'Проверка VAR' }, { anchor: { to: 'score:panel', at: 'bottom-left', self: 'top-left', offset: [0, 12] } }),
      },
      // Перерыв: часы стоят.
      break: { score: card({ period: 'Перерыв' }) },
    };
  },
  // Когда начинается каждый шаг перехода; остальные пары — всё сразу (together).
  transitions: {
    // Представление встречи уходит, карточка входит через 0.1 с после.
    'pregame -> live': sequence(0.1),
    // Карточка переходит на новые пропсы сразу, гость появляется через 0.3 с после этого.
    'live -> interview': ({ enter }) => [enter('lower', { with: 'score:update', offset: 0.3 })],
  },
});
