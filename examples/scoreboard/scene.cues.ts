// Бейдж уходит и возвращается по данным, счёт меняется: kine filmstrip examples/scoreboard/scene.cues.ts
import { cues } from '@kine/core';
import { Scoreboard } from './Scoreboard';
import scene from './scene';

export default cues(scene, ({ at }) => [
  at(0, 'play'),
  at(1, 'update', Scoreboard, { live: false }),
  at(1.6, 'update', Scoreboard, { live: true, homeScore: 3 }),
  at(2.6, 'stop'),
]);
