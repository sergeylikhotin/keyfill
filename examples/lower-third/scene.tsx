import { defineScene, signal } from '@kine/core';
import { LowerThird } from './LowerThird';

const live = signal(true);

export default defineScene({
  stage: { ref: [1920, 1080] },
  fps: 50,
  root: () => <LowerThird name="Анна Смирнова" role="Ведущая" live={live} tags={[]} />,
});
