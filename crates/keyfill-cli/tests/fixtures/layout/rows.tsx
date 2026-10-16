// Анимация геометрии: строки списка переезжают (transition.layout), контейнер меняет высоту.
import { Box, For, defineScene, fade, hex, prop, tween } from '@keyfill/core';

const move = tween(1, 'linear');

export default defineScene({
  stage: { ref: [640, 360] },
  fps: 25,
  props: { rows: prop.list(String) },
  defaults: { rows: ['a', 'b', 'c'] },
  root: (p) => (
    <Box
      anchor={{ to: 'stage', at: 'top-left' }}
      size={{ w: 200 }}
      look={{ background: hex('#202020'), transition: { layout: move } }}
      must={{ fit: true }}
    >
      <For each={p.rows}>
        {() => (
          <Box
            size={{ w: 200, h: 40 }}
            look={{ background: hex('#808080'), transition: { layout: move } }}
            presence={{ out: fade([1, 0], tween(0.5)) }}
          />
        )}
      </For>
    </Box>
  ),
});
