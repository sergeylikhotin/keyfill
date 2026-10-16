// Случаи, которые обязаны упасть: tests/cli.rs проверяет отчёт keyfill test.
// Имя не *.keyfill.test.ts, чтобы `keyfill test` в корне репозитория их не подхватывал.
import { test } from '@keyfill/core';
import overflow from '../../../../../examples/scoreboard/overflow';
import lower from '../../../../../examples/lower-third/scene';

// must.fit в dev: clean падает.
test('ошибка must', { scene: overflow });

// Снапшот в __snapshots__ намеренно испорчен: имя в нём другое.
test('снапшот разошёлся', { scene: lower, expect: { snapshot: 'dump' } });

// Неизвестный маркер — ошибка момента, остальные моменты случая проверяются.
test('неверный момент', { scene: lower, t: ['nope', 'settled'] });

// Данные не по схеме — ошибка всего случая.
test('неверные пропсы', { scene: lower, props: { nam: 'x' } as never });
