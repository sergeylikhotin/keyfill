# config — keyfill.config.ts

Значения по умолчанию для команд проекта. keyfill ищет `keyfill.config.ts` вверх от каталога запуска;
флаги перекрывают конфиг. В `--json` путь конфига — поле `config` конверта.

    import { defineConfig } from '@keyfill/core';

    export default defineConfig({
      mode: 'dev',                 // --mode
      bg: '#101820',               // --bg: подложка эфира, нужна для проверки контраста
      seed: 1,                     // --seed
      now: 0,                      // --now: Date.now() сцены, мс
      out: 'out',                  // каталог для render, filmstrip, video без -o и для keyfill view
      scenes: {                    // именованные сцены: keyfill render lower, keyfill check score
        lower: 'examples/lower-third/scene.cues.ts',
        score: 'examples/scoreboard/scene.cues.ts',
      },
    });

- Пути — от каталога конфига. С `out` файл по умолчанию повторяет путь сцены:
  `keyfill render examples/lower-third/scene.tsx` → `out/examples/lower-third/scene.png`. Без `out` —
  рядом со сценой.
- Имя сцены годится везде, где нужен `<entry>`; файл с таким путём важнее имени.
- tsc проверяет ключи и типы; неизвестный ключ — ошибка вызова (код 2) с именем ключа.
- `keyfill test` идёт из каталога конфига: пути в снапшотах — от него.
