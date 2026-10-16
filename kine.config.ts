// Значения по умолчанию для kine в этом репозитории; флаги их перекрывают (kine docs config).
import { defineConfig } from '@kine/core';

export default defineConfig({
  // render, filmstrip и video без -o пишут сюда: out/examples/lower-third/scene.png.
  out: 'out',
  // kine render lower, kine check score.
  scenes: {
    lower: 'examples/lower-third/scene.cues.ts',
    score: 'examples/scoreboard/scene.cues.ts',
  },
});
