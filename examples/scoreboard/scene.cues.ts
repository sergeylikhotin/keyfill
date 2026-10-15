// Эфирный сценарий табло: kine filmstrip examples/scoreboard/scene.cues.ts --n 16
import { cues } from '@kine/core';
import { Scoreboard } from './Scoreboard';
import scene from './scene';

export default cues(scene, ({ at }) => [
  at(0, 'play'), // появление, затем стоп на ready
  at(1.2, 'next'), // дальше: цикл loop..out
  at(1.6, 'update', Scoreboard, { live: false }), // бейдж уходит через свой out
  at(2.2, 'invoke', Scoreboard, 'goal', { side: 'home' }), // счёт 3:1, вспышка, возврат в цикл
  at(3.0, 'update', Scoreboard, { live: true }),
  at(4.2, 'stop'), // прыжок на out с текущих значений, затем табло снято
]);
