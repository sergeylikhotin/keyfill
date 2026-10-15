import { defineScene, signal } from '@kine/core';
import { LowerThird } from './LowerThird';

const live = signal(true);

export default defineScene({
  stage: { ref: [1920, 1080] },
  fps: 50,
  fonts: ['../../assets/fonts/inter/Inter-Regular.otf', '../../assets/fonts/inter/Inter-SemiBold.otf'],
  systemFonts: false,
  root: () => <LowerThird name="Анна Смирнова" role="Ведущая" live={live} tags={[]} />,
});
