// Канал эфира: каталог графики для слоёв. Что и когда стоит на слоях, решают команды —
// журнал show.journal.jsonl (keyfill render examples/channel/show.journal.jsonl), с M4b — пульт.
import { defineChannel } from '@keyfill/core';
import interRegular from '../../assets/fonts/inter/Inter-Regular.otf';
import interSemiBold from '../../assets/fonts/inter/Inter-SemiBold.otf';
import { LowerThird } from '../lower-third/LowerThird';
import { Scoreboard } from '../scoreboard/Scoreboard';
import scoreboardMotion from '../scoreboard/Scoreboard.motion';

export default defineChannel({
  stage: { ref: [1920, 1080] },
  fps: 50,
  fonts: [interRegular, interSemiBold],
  systemFonts: false,
  timelines: [scoreboardMotion],
  components: [Scoreboard, LowerThird],
});
