import { defineScene } from '@keyfill/core';
import interRegular from '../../assets/fonts/inter/Inter-Regular.otf';
import interSemiBold from '../../assets/fonts/inter/Inter-SemiBold.otf';
import { Standings, teams } from './Standings';

export default defineScene({
  stage: { ref: [1920, 1080] },
  fps: 50,
  fonts: [interRegular, interSemiBold],
  systemFonts: false,
  // Данные выпуска: keyfill render scene.tsx --props round.json; в сценарии — at(t, 'update', { teams }).
  props: { teams },
  defaults: {
    teams: [
      { id: 'zen', name: 'Зенит', points: 41 },
      { id: 'kra', name: 'Краснодар', points: 39 },
      { id: 'din', name: 'Динамо', points: 35 },
    ],
  },
  root: (p) => <Standings teams={p.teams} />,
});
