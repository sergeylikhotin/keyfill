// Тесты плашки: kine test examples/lower-third. Снапшоты — в __snapshots__/lower-third,
// перезапись после намеренной правки — kine test examples/lower-third --update.
import { test } from '@kine/core';
import scene from './scene';
import script from './scene.cues';

// Кадр в эфире на подложке эфира (нужна для проверки контраста): дамп и пиксели.
// PNG в половину размера — файл вчетверо меньше.
test('в эфире', { scene, bg: '#101820', scale: 0.5, expect: { snapshot: ['dump', 'png'] } });

// Длинное имя: плашка растёт до ширины titleSafe, дальше кегль ужимается (fit 56→40.5), а не переполняет её.
test('длинное имя', {
  scene,
  props: { name: 'Константин Константинопольский-Преображенский-Новосельцев-Задунайский' },
  bg: '#101820',
  expect: { snapshot: 'dump' },
});

// Весь сценарий без ошибок; снапшоты — после смены данных и посреди ухода.
test('сценарий', { scene: script, t: ['0..5', 2.6, 3.6], expect: { snapshot: 'dump' } });
