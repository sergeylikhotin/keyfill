import { defineScene, signal } from '@kine/core';
import interRegular from '../../assets/fonts/inter/Inter-Regular.otf';
import interSemiBold from '../../assets/fonts/inter/Inter-SemiBold.otf';
import { LowerThird } from './LowerThird';

const live = signal(true);

export default defineScene({
  stage: { ref: [1920, 1080] },
  fps: 50,
  fonts: [interRegular, interSemiBold],
  systemFonts: false,
  root: () => <LowerThird name="Анна Смирнова" role="Ведущая" live={live} tags={[]} />,
});
