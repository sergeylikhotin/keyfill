// Сценарий для рендера в файл и проверок: kine filmstrip examples/lower-third/scene.cues.ts
import { cues } from '@kine/core';
import { LowerThird } from './LowerThird';
import scene from './scene';

export default cues(scene, ({ at }) => [
  at(0, 'play'),
  // Эфир кончился: подложка плавно гаснет (transition), точка уходит через свой out.
  at(1.5, 'update', LowerThird, { live: false }),
  at(2.5, 'update', LowerThird, { name: 'Анна Смирнова-Ковалевская' }),
  at(3.5, 'stop'),
]);
