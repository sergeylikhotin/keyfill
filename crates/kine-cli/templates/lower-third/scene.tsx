import { defineScene } from '@kine/core';
import interRegular from '__FONTS__/Inter-Regular.otf';
import interSemiBold from '__FONTS__/Inter-SemiBold.otf';
import { __Name__ } from './__Name__';

export default defineScene({
  stage: { ref: [1920, 1080] },
  fps: 50,
  // Шрифты — файлами: рендер одинаковый на любой машине.
  fonts: [interRegular, interSemiBold],
  systemFonts: false,
  // Данные выпуска: kine render __entry__ --set name="Иван Петров" или --props data.json.
  props: { name: String, role: String, live: Boolean },
  defaults: { name: 'Анна Смирнова', role: 'Ведущая', live: true },
  root: (p) => <__Name__ name={p.name} role={p.role} live={p.live} />,
});
