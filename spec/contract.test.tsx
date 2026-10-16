// Контракт API: каждая строка с @ts-expect-error — ошибка, которую tsc обязан поймать
// до рендера. Если ошибка перестаёт ловиться, tsc ругается на неиспользованный expect-error.
import { Box, For, Text, clock, component, count, cues, defineChannel, defineConfig, defineScene, hex, overlap, prop, sequence, stagger, test, timeline, tween } from '@keyfill/core';
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

// Анимация геометрии — изинг, а не флаг; animate по layout не бывает (это не канал).
<Box look={{ transition: { layout: tween(0.3) } }} />;
// @ts-expect-error layout — изинг, а не boolean
<Box look={{ transition: { layout: true } }} />;
timeline(LowerThird, ({ at }) => ({
  // @ts-expect-error layout не канал таймлайна
  keys: [at(0, 'panel', { layout: 1 })],
}));

// Состояния канала: слой — из layers, пропсы — по типам компонента, данные — из props канала.
defineChannel({
  stage: { ref: [1920, 1080] },
  fps: 50,
  props: { guest: prop.string({ default: '' }), score: prop.number({ default: 0 }) },
  layers: { lower: 20 },
  states: (d) => ({
    on: { lower: LowerThird.with({ name: d.guest, role: 'Гость', live: true }) },
    // @ts-expect-error score — число, а name — строка
    wrong: { lower: LowerThird.with({ name: d.score, role: 'Гость', live: true }) },
    // @ts-expect-error в данных канала нет guestRole
    missing: { lower: LowerThird.with({ name: d.guest, role: d.guestRole, live: true }) },
  }),
});

defineChannel({
  stage: { ref: [1920, 1080] },
  fps: 50,
  layers: { lower: 20 },
  // @ts-expect-error слоя score нет в layers
  states: () => ({ bad: { score: LowerThird.with({ name: 'x', role: 'Гость', live: true }) } }),
});

// Место экземпляра в состоянии: кадр, safe area или 'слой:часть'.
LowerThird.with({ name: 'x', role: 'y', live: true }, { anchor: { to: 'stage', at: 'center' } });
LowerThird.with({ name: 'x', role: 'y', live: true }, { anchor: { to: 'score:panel', at: 'bottom-left', self: 'top-left', offset: [0, 12] } });
// @ts-expect-error parent у экземпляра на слое не бывает
LowerThird.with({ name: 'x', role: 'y', live: true }, { anchor: { to: 'parent', at: 'center' } });

// Хореография: шаги по слоям из layers, якоря — 'слой:in|out|update'.
defineChannel({
  stage: { ref: [1920, 1080] },
  fps: 50,
  layers: { lower: 20, score: 10 },
  states: () => ({ a: { lower: LowerThird.with({ name: 'x', role: 'y', live: true }) }, b: {} }),
  transitions: {
    default: overlap(0.15),
    'a -> b': sequence(0.1),
    'b -> a': ({ enter, update }) => [enter('lower', { after: 'score:out', offset: -0.1 }), update('score', { at: 0.2 })],
    // @ts-expect-error слоя replay нет в layers
    '* -> a': ({ enter }) => [enter('replay')],
    // @ts-expect-error якорь — 'слой:in|out|update'
    'a -> *': ({ out }) => [out('lower', { after: 'score:hold' })],
  },
});

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
  // @ts-expect-error update без пропсов
  at(3, 'update'),
  // @ts-expect-error у demo нет пропсов сцены
  at(3, 'update', { name: 'Иван' }),
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

// Пропсы сцены: defaults и update в сценарии проверяются по схеме, root получает аксессоры.
const show = defineScene({
  stage: { ref: [1920, 1080] },
  fps: 50,
  props: { guest: String, score: Number },
  defaults: { guest: 'Анна' },
  root: (p) => <LowerThird name={p.guest} role={() => String(p.score())} live={true} tags={[]} />,
});
cues(show, ({ at }) => [
  at(1, 'update', { score: 2 }),
  // @ts-expect-error score — число
  at(1, 'update', { score: '2' }),
  // @ts-expect-error у сцены нет пропса guests
  at(1, 'update', { guests: 'Иван' }),
]);
defineScene({
  stage: { ref: [1920, 1080] },
  fps: 50,
  props: { guest: String },
  // @ts-expect-error значение по умолчанию — строка
  defaults: { guest: 5 },
  root: () => <Box />,
});
defineScene({
  stage: { ref: [1920, 1080] },
  fps: 50,
  props: { guest: String },
  // @ts-expect-error guest — аксессор, а не строка
  root: (p) => <LowerThird name={p.guest.toUpperCase()} role="" live={true} tags={[]} />,
});

// Тесты: пропсы случая проверяются по схеме сцены, в том числе через сценарий.
const showCues = cues(show, ({ at }) => [at(0, 'play')]);
test('пропсы по схеме', { scene: show, props: { guest: 'Иван' }, t: ['settled', 1.2], expect: { snapshot: ['dump', 'png'] } });
test('через сценарий', { scene: showCues, props: { score: 3 } });
// @ts-expect-error score — число
test('тип пропса', { scene: show, props: { score: '3' } });
// @ts-expect-error у сцены нет пропса guests
test('лишний пропс', { scene: showCues, props: { guests: 'Иван' } });
// @ts-expect-error снапшот — dump или png
test('вид снапшота', { scene: show, expect: { snapshot: 'svg' } });

// Конфиг: только известные ключи.
defineConfig({ mode: 'air', bg: '#101820', out: 'out', scenes: { lower: 'examples/lower-third/scene.cues.ts' } });
// @ts-expect-error профилей выходов нет
defineConfig({ profiles: {} });
// @ts-expect-error подложка — #RRGGBB
defineConfig({ bg: 'black' });

// Типы пропсов в рантайме: tsc выводит тип из описания, пропсы с default необязательны.
const Card2 = component({
  name: 'Card2',
  props: {
    title: prop.string({ description: 'Заголовок' }),
    side: prop.enum(['home', 'away']),
    score: prop.number({ min: 0, int: true, default: 0 }),
    players: prop.list(prop.object({ name: String, number: prop.number({ default: 0 }) }), { default: [] }),
    photo: prop.image(),
    extra: prop.json<{ a: number }>({ default: { a: 1 } }),
  },
  view: (p) => {
    const _s: 'home' | 'away' = p.side();
    const _n: number = p.players()[0].number;
    return <Text>{p.title}</Text>;
  },
});
<Card2 title="x" side="home" photo="p.png" />;
<Card2 title="x" side="away" photo="p.png" score={2} players={[{ name: 'Иван' }]} extra={{ a: 2 }} />;
// @ts-expect-error side — home или away
<Card2 title="x" side="left" photo="p.png" />;
// @ts-expect-error нет обязательного photo
<Card2 title="x" side="home" />;
// @ts-expect-error у игрока нет имени
<Card2 title="x" side="home" photo="p.png" players={[{ number: 7 }]} />;
// @ts-expect-error default enum — одно из значений
prop.enum(['home', 'away'], { default: 'left' });
// @ts-expect-error default числа — число
prop.number({ default: '0' });
// @ts-expect-error голый prop<T>() заменён на prop.json<T>()
prop<string[]>();

// Канал: каталог — компоненты из component(), не строки.
defineChannel({ stage: { ref: [1920, 1080] }, fps: 50, components: [LowerThird, Scoreboard, Card2] });
// @ts-expect-error каталог — компоненты, а не имена
defineChannel({ stage: { ref: [1920, 1080] }, fps: 50, components: ['LowerThird'] });

// Списки по данным: у объектов ключ by обязателен, у строк и чисел — само значение.
const rowsData = [{ id: 'zen', name: 'Зенит' }];
<Box>
  <For each={rowsData} by={(r) => r.id}>{(r, i) => <Text>{() => `${i() + 1}. ${r().name}`}</Text>}</For>
  <For each={['a', 'b']}>{(s) => <Text>{s}</Text>}</For>
</Box>;
// @ts-expect-error у объектов нужен by
<For each={rowsData}>{(r) => <Text>{() => r().name}</Text>}</For>;
// @ts-expect-error ключ — строка или число
<For each={rowsData} by={(r) => r}>{(r) => <Text>{() => r().name}</Text>}</For>;
// @ts-expect-error у элемента нет поля title: тип выводится из each
<For each={rowsData} by={(r) => r.id}>{(r) => <Text>{() => r().title}</Text>}</For>;
