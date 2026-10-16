// Канал со списком, строки которого переезжают: живые команды и журнал дают одну геометрию.
import { Box, For, component, defineChannel, fade, hex, prop, spring, tween } from '@keyfill/core';

const Rows = component({
  name: 'Rows',
  props: { rows: prop.list(String) },
  view: ({ rows }) => (
    <Box anchor={{ to: 'stage', at: 'top-left' }} size={{ w: 200 }} look={{ background: hex('#202020'), transition: { layout: spring.gentle } }}>
      <For each={rows}>
        {() => (
          <Box
            size={{ w: 200, h: 40 }}
            look={{ background: hex('#808080'), transition: { layout: spring.snappy } }}
            presence={{ out: fade([1, 0], tween(0.3)) }}
          />
        )}
      </For>
    </Box>
  ),
});

export default defineChannel({ stage: { ref: [640, 360] }, fps: 25, components: [Rows] });
