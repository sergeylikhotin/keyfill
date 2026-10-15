// Название, которое не влезает даже в минимальный кегль: в dev это ошибка must.fit
// с подсказкой, какой шаг else её исправит; в air (--mode air) шаг применяется сам.
import { defineScene, signal } from '@kine/core';
import interRegular from '../../assets/fonts/inter/Inter-Regular.otf';
import interSemiBold from '../../assets/fonts/inter/Inter-SemiBold.otf';
import { Scoreboard } from './Scoreboard';

export default defineScene({
  stage: { ref: [1920, 1080] },
  fps: 50,
  fonts: [interRegular, interSemiBold],
  systemFonts: false,
  root: () => (
    <Scoreboard
      home="Динамо Москва"
      away="Футбольный клуб Торпедо имени Эдуарда Стрельцова"
      homeScore={0}
      awayScore={0}
      period="1-й тайм"
      clockFrom={12 * 60}
      clockRunning={false}
      live={signal(false)}
    />
  ),
});
