# motion — presence, transition, числа и часы

Три уровня движения, все детерминированы:

- `transition` (в `look` или на узле) — как значение интерполируется при смене данных:
  `look={{ background: on(live, red), transition: { background: tween(0.3), all: spring.gentle } }}`;
- `presence` (на узле или в `component()`) — появление `in`, цикл `loop`, уход `out`;
- `timeline` (`*.motion.ts`) — постановка с маркерами и остановками (kine docs timeline).

    presence={{
      in: seq(fade([0, 1], tween(0.2)), slide({ y: [24, 0] }, spring.snappy)),
      loop: pulse({ period: 1.2, opacity: [1, 0.6] }),
      out: fade([1, 0], tween(0.25, 'in')),
    }}

Конструкторы: `animate(targets, easing)`, `fade([a, b])`, `slide({ x, y })`, `seq(...)`, `par(...)`,
`delay(s, m)`, `pulse({ period, opacity, scale })`, `stagger(step, m, { reverse })`.
Каналы: `x`, `y`, `rotate`, `scale`, `opacity`, `reveal`, `width`, `height`, `radius`, `background`,
`color`. Изинги: `tween(duration, 'linear'|'in'|'out'|'inOut'|[x1,y1,x2,y2])`, `spring.snappy`,
`spring.gentle`, `spring.bouncy`, `spring({ stiffness, damping, mass })`.

Слой движения лежит поверх `look`: `x`, `y`, `rotate` — сдвиг, `scale`, `opacity` — множитель,
остальное перекрывает значение `look`.

Жизненный цикл: `loaded` → `play` → `in` → `loop` → `stop` → `out` → `removed`. Узел уходит только
после конца `out`. Любое движение стартует из текущего значения; узел, которого не было на экране,
играет `in` с объявленных значений.

Каскад: `presence={{ in: stagger(0.06, fade([0, 1])) }}` на контейнере — каждому видимому ребёнку
со сдвигом `i·step`.

Числа и часы ведёт ядро, JS на кадрах не вызывается:

    <Text>{count(score, { easing: tween(0.6), format: '0' })}</Text>
    <Text>{clock({ from: 67 * 60, running, format: 'mm:ss' })}</Text>

Фаза движения узла — в дампе (`[in 78%]`, `[timeline hold τ=0.7 на ready]`) и в `kine inspect`.
