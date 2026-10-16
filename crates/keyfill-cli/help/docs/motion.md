# motion — presence, transition, числа и часы

Три уровня движения, все детерминированы:

- `transition` (в `look` или на узле) — как значение интерполируется при смене данных:
  `look={{ background: on(live, red), transition: { background: tween(0.3), all: spring.gentle } }}`;
- `presence` (на узле или в `component()`) — появление `in`, цикл `loop`, уход `out`;
- `timeline` (`*.motion.ts`) — постановка с маркерами и остановками (keyfill docs timeline).

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

Анимация геометрии — `transition: { layout: spring.snappy }`. Когда раскладка перестроилась
(данные, сосед пришёл или ушёл, перестановка в `For`, смена размера текста), узел едет к новому
месту, а `Box` — ещё и к новому размеру. Текст едет, но размер не меняет: на кадрах переезда он не
переразбивается. Без `layout` место меняется скачком; `all` геометрию не включает. Узел без своего
`layout` едет вместе с родителем. Растущая подложка с `clip: true` открывает содержимое по мере
роста, без неё текст на первых кадрах шире подложки.

Раскладка сверяется в моменты команд и уходов, а не на кадрах, поэтому кадр зависит только от
момента. Прерывание — как у остального движения: переезд начинается оттуда, где узел виден,
пружина сохраняет скорость. `must` проверяет место в раскладке, а не промежуточный кадр. В дампе
узел в пути — `(видно → место в раскладке)  [layout 40%]`, в `inspect` — строка `layout`.

Жизненный цикл: `loaded` → `play` → `in` → `loop` → `stop` → `out` → `removed`. Узел уходит только
после конца `out`. Любое движение стартует из текущего значения; узел, которого не было на экране,
играет `in` с объявленных значений.

Каскад: `presence={{ in: stagger(0.06, fade([0, 1])) }}` на контейнере — каждому видимому ребёнку
со сдвигом `i·step`.

Числа и часы ведёт ядро, JS на кадрах не вызывается:

    <Text>{count(score, { easing: tween(0.6), format: '0' })}</Text>
    <Text>{clock({ from: 67 * 60, running, format: 'mm:ss' })}</Text>

Фаза движения узла — в дампе (`[in 78%]`, `[timeline hold τ=0.7 на ready]`) и в `keyfill inspect`.
