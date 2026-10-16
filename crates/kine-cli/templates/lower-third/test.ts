// Тесты сцены: kine test __dir__. Снапшоты — в __snapshots__ рядом, обновить — --update.
import { test } from '@kine/core';
import scene from './scene';
import script from './scene.cues';

// Кадр в эфире на подложке (нужна для проверки контраста): дамп и пиксели.
test('в эфире', { scene, bg: '#101820', scale: 0.5, expect: { snapshot: ['dump', 'png'] } });

// Длинное имя: плашка растёт до края titleSafe, дальше кегль ужимается (fit), а не переполняет её.
test('длинное имя', { scene, props: { name: 'Константин Константинопольский-Преображенский-Новосельцев-Задунайский' }, bg: '#101820', expect: { snapshot: 'dump' } });

// Весь сценарий без ошибок.
test('сценарий', { scene: script, t: ['0..3'] });
