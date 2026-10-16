// Таблица по ходу тура: keyfill filmstrip examples/standings/scene.cues.ts --n 12
import { cues } from '@keyfill/core';
import scene from './scene';

export default cues(scene, ({ at }) => [
  at(0, 'play'), // три строки въезжают каскадом
  // Зенит +3 (число едет), Динамо выбывает (уходит каскадом out), Спартак появляется вторым.
  at(1.5, 'update', {
    teams: [
      { id: 'zen', name: 'Зенит', points: 44 },
      { id: 'spa', name: 'Спартак', points: 40 },
      { id: 'kra', name: 'Краснодар', points: 39 },
    ],
  }),
  // Спартак выходит на первое место, Динамо возвращается последним.
  at(3, 'update', {
    teams: [
      { id: 'spa', name: 'Спартак', points: 46 },
      { id: 'zen', name: 'Зенит', points: 44 },
      { id: 'kra', name: 'Краснодар', points: 39 },
      { id: 'din', name: 'Динамо', points: 36 },
    ],
  }),
  at(4.5, 'stop'),
]);
