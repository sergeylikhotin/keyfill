// Канал для тестов: команда, которая падает посреди исполнения, и компонент без presence.
import { Box, Text, component, defineChannel, prop } from '@keyfill/core';

const Counter = component({
  name: 'Counter',
  props: { n: prop.number({ int: true, default: 0 }), label: prop.string({ default: 'n' }) },
  view: ({ n, label }) => (
    <Box anchor={{ to: 'titleSafe', at: 'top-left' }} look={{ background: '#202020', padding: 8 }}>
      <Text look={{ size: 32, color: '#ffffff' }}>{() => `${label()} ${n()}`}</Text>
    </Box>
  ),
  commands: {
    // Сначала меняет данные, потом падает: частичный эффект остаётся, как в эфире.
    bump: {
      params: { by: prop.number({ int: true }), fail: prop.boolean({ default: false }) },
      run: ({ by, fail }, ctx) => {
        ctx.set({ n: ctx.props.n() + by });
        if (fail) throw new Error('bump упал');
      },
    },
  },
});

export default defineChannel({ stage: { ref: [640, 360] }, fps: 25, components: [Counter] });
