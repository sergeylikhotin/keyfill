import { defineScene, signal } from '@keyfill/core';
import interRegular from '../../assets/fonts/inter/Inter-Regular.otf';
import interSemiBold from '../../assets/fonts/inter/Inter-SemiBold.otf';
import { LowerThird } from './LowerThird';

const live = signal(true);

export default defineScene({
  stage: { ref: [1920, 1080] },
  fps: 50,
  fonts: [interRegular, interSemiBold],
  systemFonts: false,
  // Данные выпуска: keyfill render scene.tsx --set name="Иван Петров" или --props guest.json.
  props: { name: String, role: String },
  defaults: { name: 'Анна Смирнова', role: 'Ведущая' },
  root: (p) => <LowerThird name={p.name} role={p.role} live={live} tags={[]} />,
});
