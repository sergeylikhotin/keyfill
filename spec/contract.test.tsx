// Контракт API: каждая строка с @ts-expect-error — ошибка, которую tsc обязан поймать
// до рендера. Если ошибка перестаёт ловиться, tsc ругается на неиспользованный expect-error.
import { Box, Text, clock, component, count, cues, defineScene, hex, stagger, timeline, tween } from '@kine/core';
import { LowerThird } from '../examples/lower-third/LowerThird';
import { Scoreboard } from '../examples/scoreboard/Scoreboard';

// Таймлайн адресует несуществующую часть.
timeline(LowerThird, ({ at }) => ({
  // @ts-expect-error 'logo' нет в parts компонента LowerThird
  keys: [at(0, 'logo', { opacity: [0, 1] })],
}));

// Текстовые свойства на Box: наследования нет, значит и смысла нет.
// @ts-expect-error font не входит в BoxLook
<Box look={{ font: { family: 'Inter' } }} />;

// Строковые микроязыки запрещены.
// @ts-expect-error shadow — структура, а не CSS-строка
<Box look={{ shadow: '0 4px 8px black' }} />;

// Анимировать можно только Animatable.
timeline(LowerThird, ({ at }) => ({
  // @ts-expect-error padding не анимируется
  keys: [at(0, 'panel', { padding: [0, 10] })],
}));

// Неверный тип пропса компонента.
// @ts-expect-error live — boolean
<LowerThird name="x" role="y" live="yes" tags={[]} />;

// Пропущен обязательный пропс.
// @ts-expect-error нет role
<LowerThird name="x" live={false} tags={[]} />;

// Часть из чужого компонента нельзя подставить по строке.
const Card = component({
  name: 'Card',
  props: { title: String },
  parts: ['root'],
  // @ts-expect-error name принимает PartRef, а не строку
  view: ({ title }) => <Box name="root"><Text>{title}</Text></Box>,
});

// Неверная кривая изинга.
// @ts-expect-error 'bounce' не EaseCurve
tween(0.2, 'bounce');

// Шрифт подключается импортом, строковый путь не принимается.
defineScene({
  stage: { ref: [1920, 1080] },
  fps: 50,
  // @ts-expect-error fonts принимает FontFile из import, а не строку
  fonts: ['../assets/fonts/inter/Inter-Regular.otf'],
  root: () => <Box />,
});

// Цвет только через конструкторы или токены.
// @ts-expect-error hex требует ведущий #
hex('FF3355');

// Сценарий: команды и пропсы проверяются по компоненту.
const demo = defineScene({ stage: { ref: [1920, 1080] }, fps: 50, root: () => <Box /> });
cues(demo, ({ at }) => [
  at(0, 'play'),
  at(1, 'update', LowerThird, { name: 'Иван' }),
  // @ts-expect-error у LowerThird нет пропса nam
  at(2, 'update', LowerThird, { nam: 'Иван' }),
  // @ts-expect-error live — boolean
  at(2, 'update', LowerThird, { live: 'yes' }),
  // @ts-expect-error неизвестная команда
  at(3, 'pause'),
  // @ts-expect-error update без компонента
  at(3, 'update'),
  at(4, 'next', Scoreboard),
  at(4, 'invoke', Scoreboard, 'goal', { side: 'home' }),
  // @ts-expect-error у Scoreboard нет команды penalty
  at(4, 'invoke', Scoreboard, 'penalty', {}),
  // @ts-expect-error side — строка
  at(4, 'invoke', Scoreboard, 'goal', { side: 1 }),
  // @ts-expect-error у LowerThird нет команд
  at(4, 'invoke', LowerThird, 'goal', {}),
]);

// Таймлайн: holds и loop ссылаются только на объявленные маркеры.
timeline(Scoreboard, ({ at }) => ({
  keys: [at(0, 'panel', { opacity: [0, 1] })],
  markers: { ready: 0.5, loop: 1, out: 2 },
  // @ts-expect-error маркера redy нет
  holds: ['redy'],
}));
timeline(Scoreboard, ({ at }) => ({
  keys: [at(0, 'panel', { opacity: [0, 1] })],
  markers: { loop: 1, out: 2 },
  // @ts-expect-error маркера lop нет
  loop: { from: 'lop', to: 'out' },
}));

// Параметры команды типизированы в её run.
component({
  name: 'Clock',
  props: { seconds: Number },
  view: () => <Box />,
  commands: {
    // @ts-expect-error by — число, у него нет toUpperCase
    add: { params: { by: Number }, run: ({ by }, ctx) => ctx.set({ seconds: by.toUpperCase() }) },
  },
});

export { Card };

// Числа и часы ведёт ядро: count принимает только число, clock — секунды.
// @ts-expect-error count — для чисел, строку анимировать нечем
count('3');
// @ts-expect-error from у часов — секунды числом
clock({ from: '67:12' });
// @ts-expect-error stagger каскадирует движение, а не изинг
stagger(0.05, tween(0.2));
