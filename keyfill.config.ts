// Значения по умолчанию для keyfill в этом репозитории; флаги их перекрывают (keyfill docs config).
import { defineConfig } from '@keyfill/core';

export default defineConfig({
  // render, filmstrip и video без -o пишут сюда: out/examples/lower-third/scene.png.
  out: 'out',
  // keyfill render lower, keyfill check score.
  scenes: {
    lower: 'examples/lower-third/scene.cues.ts',
    score: 'examples/scoreboard/scene.cues.ts',
  },
});
