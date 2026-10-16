// Таблица по ходу тура: снапшоты дампа в моменты ухода, входа и перестановки строк.
import { test } from '@keyfill/core';
import scene from './scene';
import tour from './scene.cues';

test('таблица', { scene, expect: { snapshot: ['dump', 'png'] } });
test('тур', { scene: tour, t: ['settled', 1.56, 2.5, 3.1, '0..4.82'], expect: { snapshot: 'dump' } });
test('длинное название', {
  scene,
  props: { teams: [{ id: 'kss', name: 'Крылья Советов Самара Самарская область', points: 30 }] },
  expect: { snapshot: 'dump' },
});
