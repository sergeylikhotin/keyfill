import { cues } from '@keyfill/core';
import scene from './rows';

export default cues(scene, ({ at }) => [
  at(0, 'play'),
  // a и c меняются местами, b остаётся.
  at(1, 'update', { rows: ['c', 'b', 'a'] }),
  // b уходит через out (до 3.5), потом a поднимается на его место, контейнер сжимается.
  at(3, 'update', { rows: ['c', 'a'] }),
  // Две новые строки: контейнер растёт, новые строки встают сразу.
  at(5, 'update', { rows: ['c', 'a', 'd', 'e'] }),
]);
