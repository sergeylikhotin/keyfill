// Состояние, которое нельзя построить: у компонента не передан обязательный пропс.
import { Box, Text, component, defineChannel, prop } from '@keyfill/core';

const Title = component({
  name: 'Title',
  props: { text: String, size: prop.number({ default: 32 }) },
  view: ({ text }) => (
    <Box>
      <Text>{text}</Text>
    </Box>
  ),
});

export default defineChannel({
  stage: { ref: [640, 360] },
  fps: 25,
  props: { title: prop.string({ default: 'x' }) },
  layers: { main: 10 },
  // @ts-expect-error text обязателен
  states: (d) => ({ on: { main: Title.with({ size: 40 }) }, ok: { main: Title.with({ text: d.title }) } }),
});
