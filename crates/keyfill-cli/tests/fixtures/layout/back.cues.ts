import { cues } from '@keyfill/core';
import scene from './rows';

// Перестановка прервана обратной посреди переезда.
export default cues(scene, ({ at }) => [
  at(0, 'play'),
  at(1, 'update', { rows: ['c', 'b', 'a'] }),
  at(1.6, 'update', { rows: ['a', 'b', 'c'] }),
]);
