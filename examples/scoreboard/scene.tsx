import { defineScene, signal } from '@kine/core';
import interRegular from '../../assets/fonts/inter/Inter-Regular.otf';
import interSemiBold from '../../assets/fonts/inter/Inter-SemiBold.otf';
import { Scoreboard } from './Scoreboard';

const live = signal(true);

export default defineScene({
  stage: { ref: [1920, 1080] },
  fps: 50,
  fonts: [interRegular, interSemiBold],
  systemFonts: false,
  root: () => (
    <Scoreboard
      home="Динамо Москва"
      away="Крылья Советов Самара"
      homeScore={2}
      awayScore={1}
      period="2-й тайм, 67′"
      live={live}
    />
  ),
});
