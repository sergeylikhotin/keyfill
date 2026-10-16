// Тесты табло: keyfill test examples/scoreboard. Моменты — маркеры таймлайна (keyfill schema).
import { test } from '@keyfill/core';
import overflow from './overflow';
import script from './scene.cues';

// Эфирный сценарий без ошибок; кадры на hold, после гола и посреди ухода.
test('эфирный сценарий', { scene: script, t: ['0..5', 'ready', 'goalHome+0.1', 'out+0.1'], expect: { snapshot: 'dump' } });

// Название не влезает даже в минимальный кегль: в dev это ошибка must.fit…
test('переполнение в dev', { scene: overflow, expect: { clean: false, snapshot: 'dump' } });

// …а в эфире применяется шаг else, и ошибок нет.
test('переполнение в air', { scene: overflow, mode: 'air', expect: { snapshot: 'dump' } });
