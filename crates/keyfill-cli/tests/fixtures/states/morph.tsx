// Морфинг как композиция: тот же экземпляр меняет облик (пропс) и место (якорь состояния), а
// подложка едет по transition.layout. И якорь к части экземпляра на другом слое.
import { Box, For, component, defineChannel, hex, prop, stack, tween } from '@keyfill/core';

const linear = tween(1, 'linear');

// Подложка с двумя обликами: маленький 100×50 и большой 300×150.
const Card = component({
  name: 'Card',
  props: { big: prop.boolean({ default: false }) },
  view: ({ big }) => (
    <Box anchor={{ to: 'stage', at: 'top-left' }} layout={stack()} look={{ background: hex('#3333cc'), transition: { layout: linear } }}>
      <Box when={() => !big()} size={{ w: 100, h: 50 }} />
      <Box when={big} size={{ w: 300, h: 150 }} />
    </Box>
  ),
});

// Подложка растёт по числу строк; плашка Tag в состоянии ставится под неё якорем.
const Panel = component({
  name: 'Panel',
  props: { rows: prop.list(String) },
  parts: ['panel'],
  view: ({ rows }, part) => (
    <Box name={part.panel} anchor={{ to: 'stage', at: 'top-left' }} size={{ w: 200 }} look={{ background: hex('#202020'), transition: { layout: linear } }}>
      <For each={rows}>{() => <Box size={{ w: 200, h: 40 }} />}</For>
    </Box>
  ),
});
const Tag = component({
  name: 'Tag',
  props: {},
  view: () => <Box size={{ w: 80, h: 20 }} look={{ background: hex('#33cc33') }} />,
});

export default defineChannel({
  stage: { ref: [640, 360] },
  fps: 25,
  props: { rows: prop.list(String, { default: ['a'] }) },
  layers: { main: 10, tag: 20, card: 30 },
  states: (d) => ({
    small: { card: Card.with({}) },
    big: { card: Card.with({ big: true }, { anchor: { to: 'stage', at: 'bottom-right' } }) },
    rows: {
      main: Panel.with({ rows: d.rows }),
      tag: Tag.with({}, { anchor: { to: 'main:panel', at: 'bottom-left', self: 'top-left', offset: [0, 10] } }),
    },
  }),
});
