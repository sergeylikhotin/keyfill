# scene — сцена, данные, шрифты

    import { defineScene } from '@keyfill/core';
    import inter from './fonts/Inter-Regular.otf';

    export default defineScene({
      stage: { ref: [1920, 1080] },          // опорный кадр; safe: { action: 0.93, title: 0.9 }
      fps: 50,
      fonts: [inter],                        // файлы шрифтов импортом; семейства — из файлов
      systemFonts: false,                    // для детерминированного рендера
      props: { name: String, score: Number },// данные сцены
      defaults: { name: 'Анна' },            // пропс без значения по умолчанию обязателен
      timelines: [scoreboardMotion],         // таймлайны компонентов (keyfill docs timeline)
      root: (p) => <LowerThird name={p.name} />,
    });

- Длины — в единицах опорного кадра; `--scale 2` даёт 4K из 1080p.
- Safe areas по EBU R95: `actionSafe` 93%, `titleSafe` 90%; цели для `anchor` и `must.inside`.
- Данные сцены: `--props data.json`, `--set name=Иван` (тип приводится по схеме; списки, объекты
  и `prop.json` — JSON; типы — `keyfill docs props`),
  в сценарии `at(t, 'update', { name: 'Иван' })`, в тесте `props: { ... }`. Ошибка данных — код 2
  с подсказкой `keyfill schema`.
- Внутри `root` и компонентов каждый пропс — аксессор: `p.name()`; передавай его целиком
  (`name={p.name}`), чтобы значение обновлялось.

Компоненты:

    const LowerThird = component({
      name: 'LowerThird',                    // экземпляры в дампе: LowerThird#1
      props: { name: String, live: Boolean },
      parts: ['panel', 'title'],             // адреса для таймлайна: name={part.title}
      presence: { in: fade([0, 1], tween(0.3)), out: fade([1, 0]) },
      commands: { flash: { params: {}, run: (_, ctx) => ctx.play('flash') } },
      view: (p, part) => <Box name={part.panel}><Text name={part.title}>{p.name}</Text></Box>,
    });

Компонент выполняется один раз: `live() && layer` в `view` не обновится (`keyfill explain
signal.untracked`). Реактивно — `on(live, layer)`, `when={live}`, аксессоры.

JS не видит реального времени: `Date`, `performance.now` — часы keyfill (`--now`), `Math.random` —
сид (`--seed`), таймеров нет.
