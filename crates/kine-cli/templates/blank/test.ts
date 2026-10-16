// Тесты сцены: kine test __dir__. Снапшоты — в __snapshots__ рядом, обновить — --update.
import { test } from '@kine/core';
import scene from './scene';

test('в эфире', { scene, expect: { snapshot: 'dump' } });
