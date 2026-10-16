// Сценарий для рендера в файл и проверок: keyfill filmstrip examples/lower-third/scene.cues.ts
import { cues } from '@keyfill/core';
import { LowerThird } from './LowerThird';
import scene from './scene';

export default cues(scene, ({ at }) => [
  at(0, 'play'),
  // Эфир кончился: подложка плавно гаснет (transition), точка уходит через свой out.
  at(1.5, 'update', LowerThird, { live: false }),
  // Данные сцены: update без компонента меняет её пропсы.
  at(2.5, 'update', { name: 'Анна Смирнова-Ковалевская' }),
  at(3.5, 'stop'),
]);
