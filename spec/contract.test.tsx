// Контракт API: каждая строка с @ts-expect-error — ошибка, которую tsc обязан поймать
// до рендера. Если ошибка перестаёт ловиться, tsc ругается на неиспользованный expect-error.
import { Box, Text, component, defineScene, hex, timeline, tween } from '@kine/core';
import { LowerThird } from '../examples/lower-third/LowerThird';

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

export { Card };
