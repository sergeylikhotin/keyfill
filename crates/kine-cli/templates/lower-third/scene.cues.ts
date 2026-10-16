// Сценарий эфира: kine filmstrip __cues__ --n 12
import { cues } from '@kine/core';
import scene from './scene';

export default cues(scene, ({ at }) => [
  at(0, 'play'),
  // Эфир кончился: подложка плавно меняет цвет (transition).
  at(1.5, 'update', { live: false }),
  at(2.5, 'stop'),
]);
