// Постановка табло: появление, остановка на ready до next, дыхание статуса в цикле,
// уход и сегменты для команды goal.
import { spring, timeline, tween } from '@keyfill/core';
import { Scoreboard } from './Scoreboard';

export default timeline(Scoreboard, ({ at, emit }) => ({
  keys: [
    at(0, 'panel', { opacity: [0, 1], y: [-24, 0] }, spring.snappy),
    at(0.15, 'home', { reveal: [0, 1] }, tween(0.35, 'out')),
    at(0.25, 'away', { reveal: [0, 1] }, tween(0.35, 'out')),
    at(0.4, 'status', { opacity: [0, 1] }, tween(0.25)),
    // Цикл loop..out: статус дышит, к концу отрезка возвращается к 1 — повтор без шва.
    at(1.0, 'status', { opacity: 0.55 }, tween(0.8)),
    at(1.8, 'status', { opacity: 1 }, tween(0.8)),
    // Уход по stop.
    at(2.6, 'panel', { opacity: 0, y: -24 }, tween(0.3, 'in')),
    // Сегменты команды goal: вспышка цифры счёта.
    at(3.0, 'homeScore', { scale: [1, 1.35] }, tween(0.12, 'out')),
    at(3.12, 'homeScore', { scale: 1 }, spring.bouncy),
    at(4.4, 'awayScore', { scale: [1, 1.35] }, tween(0.12, 'out')),
    at(4.52, 'awayScore', { scale: 1 }, spring.bouncy),
  ],
  markers: { ready: 0.7, loop: 1.0, out: 2.6, goalHome: 3.0, goalAway: 4.4 },
  holds: ['ready'],
  events: [emit(0.6, 'reveal-done')],
}));
