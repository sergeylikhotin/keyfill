import { cues } from '@keyfill/core';
import scene from './reenter';
export default cues(scene, ({ at }) => [at(0, 'play'), at(1, 'update', { vis: false }), at(2, 'update', { vis: true })]);
