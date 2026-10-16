# timeline — таймлайны, маркеры, команды компонента

Таймлайн — постановка для частей компонента в `*.motion.ts`; у каждого экземпляра — своё
воспроизведение. Подключается к сцене: `defineScene({ timelines: [scoreboardMotion] })`.

    export default timeline(Scoreboard, ({ at, emit }) => ({
      keys: [
        at(0, 'panel', { opacity: [0, 1], y: [24, 0] }, tween(0.4, 'out')),
        at(1.0, 'badge', { scale: 1.1 }, spring.snappy),
      ],
      markers: { ready: 0.7, loop: 1.0, out: 2.6, goalHome: 3.0, goalAway: 4.4 },
      loop: { from: 'loop', to: 'out', mode: 'repeat' },   // 'pingpong', count: n
      holds: ['ready'],                                    // стоп до next
      events: [emit(0.6, 'reveal-done')],
    }));

Части — из `parts` компонента; tsc сверяет имя части в `at(t, 'panel', ...)`.

Воспроизведение:

- `play` — от 0 вперёд; на маркере из `holds` — стоп до `next`; отрезок цикла повторяется до `next`;
  за маркер `out` обычное воспроизведение не заходит. Без `loop`, если есть маркеры `loop` и `out`,
  повторяется `loop..out`.
- `next` снимает текущую остановку; нет остановки — событие `ignored`.
- `stop` — прыжок на `out`, воспроизведение до следующего маркера или конца, затем экземпляр снят.
- `ctx.play('goalHome')` в команде компонента — сегмент от маркера до следующего, затем возврат
  туда, где воспроизведение было.

Прыжки продолжают каналы с текущих значений. Маркер, на котором кончается сегмент или виток,
не считается пройденным. `presence` корня компонента с таймлайном не исполняется.

Команды компонента — действия с параметрами для пульта и сценария:

    commands: {
      goal: {
        params: { side: String },
        run: ({ side }, ctx) => { ctx.set({ homeScore: ctx.props.homeScore() + 1 }); ctx.play('goalHome'); ctx.emit('goal'); },
      },
    },

`ctx`: `props`, `set(patch)`, `play(marker?)`, `next()`, `stop()`, `emit(name, data?)`. В сценарии —
`at(2.2, 'invoke', Scoreboard, 'goal', { side: 'home' })`.

Маркеры, holds и цикл каждого компонента — `keyfill schema <сцена>`; маркер в `--t` — момент его
первого прохождения: `--t goalHome+0.1`.
