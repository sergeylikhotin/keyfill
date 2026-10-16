# cues — сценарий команд

Сценарий — то, что делает оператор эфира, записанное по времени: `*.cues.ts`.

    import { cues } from '@keyfill/core';
    import scene from './scene';
    import { Scoreboard } from './Scoreboard';

    export default cues(scene, ({ at }) => [
      at(0, 'play'),                                    // in у узлов с presence, play таймлайнов
      at(1.2, 'next'),                                  // снять hold или выйти из цикла
      at(1.6, 'update', Scoreboard, { live: false }),   // пропсы экземпляров компонента
      at(2.0, 'update', { name: 'Иван' }),              // пропсы самой сцены
      at(2.2, 'invoke', Scoreboard, 'goal', { side: 'home' }),  // команда компонента
      at(4.2, 'stop'),                                  // out; сцена снимается после последнего out
    ]);

- tsc проверяет команды, компоненты, пропсы и параметры команд.
- С компонентом `play`, `next`, `stop` адресуют только его экземпляры: `at(1, 'stop', Scoreboard)`.
- Команды применяются на границе кадра (время округляется вверх).
- Без сценария сцена получает `play` в 0.
- Короткая форма для CLI дополняет сценарий: `--cue "0 play; 125f stop; 2 next Scoreboard"`
  (только play, stop, next).

Прерывания детерминированы: `stop` посреди `in` начинает `out` с текущих значений, `update`
посреди перехода перенацеливает его, пружина сохраняет скорость.

События (`state`, `marker`, `emit`, `ignored`) видны в строке `events:` дампа; команда без эффекта
(`next` без остановки) — событие `ignored`.
