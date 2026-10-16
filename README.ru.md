# keyfill

[English](README.md) · **Русский**

Нативный движок рендера интерфейсов и эфирной графики: сцена описывается в TSX, keyfill рисует её без браузера. Основной автор сцен — AI-агент, поэтому движок отдаёт не только картинку, но и структуру: дамп сцены, происхождение каждого значения, диагностики с местом в исходнике.

Замысел, принципы и план — в [DESIGN.md](DESIGN.md). Контракт API — [spec/keyfill.d.ts](spec/keyfill.d.ts).

## Установка в проект

```bash
npm i -D @keyfill/cli typescript
```

```bash
npx keyfill new lower-third
```

`@keyfill/cli` ставит команду `keyfill`, `@keyfill/core` (API сцен, рантайм, бандлер), esbuild и нативный бинарь под платформу. `keyfill new` создаёт сцену из шаблона, а в новом проекте ещё `tsconfig.json`, `keyfill.config.ts` и шрифт Inter; дальше `npx keyfill check lower-third`, `npx keyfill test`, `npx tsc -p .`. Окружение проверяет `npx keyfill doctor`. Пакеты собираются из репозитория (`npm run npm:pack`, проверка установки в чистый проект — `npm run npm:smoke`); в npm они пока не опубликованы.

## Быстрый старт в репозитории

Нужны Rust (stable, MSVC на Windows) и Node.

```bash
npm install
cargo build
```

```bash
cargo run -q -p keyfill-cli -- render examples/lower-third/scene.tsx -o out/lower-third.png --bg "#2b3340" --dump
```

Видео и просмотрщик (рендер видео лучше собирать в release):

```bash
cargo run -q --release -p keyfill-cli -- video examples/scoreboard/scene.cues.ts -o out/scoreboard.webm
```

```bash
cargo run -q --release -p keyfill-cli -- view examples/scoreboard/scene.cues.ts
```

## Команды

| Команда | Что делает |
|---|---|
| `keyfill render <entry> [-o out.png] [--scale N] [--dump]` | PNG кадра; без `--bg` — с прозрачностью |
| `keyfill dump <entry> [--path <узел>] [--depth N]` | Дерево сцены: адреса узлов, геометрия, стили, фазы движения, диагностики; `--path` — поддерево узла или экземпляра, `--depth` — глубина |
| `keyfill inspect <entry> <path>` | Один узел: значения, токены и слои, откуда они пришли, каналы движения, исходник |
| `keyfill check <entry> [--t a..b]` | Нарушения `must` и правил на каждом кадре диапазона (по умолчанию — весь сценарий): одна запись на правило и узел, с первым и последним кадром; код выхода 1 при ошибках |
| `keyfill schema <entry>` | Пропсы сцены, компоненты с пропсами, частями и командами, маркеры таймлайнов, команды сценария |
| `keyfill filmstrip <entry> [--t a..b] [-o strip.png] [--n 8] [--cols 4] [--tile 480] [--dump]` | Раскадровка: n моментов диапазона одной картинкой, с `--dump` — дамп каждого |
| `keyfill video <entry> [--t a..b] [-o out.mov] [--codec prores\|qtrle\|vp9\|h264\|png] [--scale N]` | Видео через ffmpeg: `.mov` — ProRes 4444 с альфой, `.webm` — VP9 с альфой, `.mp4` — H.264 на подложке, каталог — PNG; побайтно воспроизводимо |
| `keyfill view [entry] [--dir out] [--port 4747]` | Веб-просмотрщик на localhost: галерея всех выводов с автообновлением и живая сцена с ползунком времени, дампом, событиями и inspect по клику |
| `keyfill test [файлы\|каталоги] [--update] [--name часть]` | Случаи `*.keyfill.test.ts`: нет ошибок (`clean`), снапшоты дампа (разница по узлам) и пикселей (допуск, маска отличий); код выхода 1, если случай упал |
| `keyfill perf <entry> [--t a..b] [--scale N]` | Время кадра по стадиям (script, motion, layout, raster) против бюджета `1/fps` |
| `keyfill explain [правило]` | Что значит правило диагностики и как его исправить; без аргумента — все правила |
| `keyfill docs [тема]` | Встроенная справка: время, сцена, сценарий, look, раскладка, must, движение, таймлайны, тесты, конфиг |
| `keyfill new <имя> [--template lower-third\|blank]` | Каталог сцены из шаблона; `tsconfig.json`, `keyfill.config.ts`, `.gitignore` и шрифт — если их ещё нет |
| `keyfill doctor` | node, `@keyfill/core`, esbuild, ffmpeg, конфиг, typescript, кэш и пробная сборка сцены |
| `keyfill cache [clean]` | Кэш бандлов: каталог и размер или очистка |

`<entry>` — сцена (`defineScene`), сценарий `*.cues.ts` (`cues(scene, ({ at }) => [...])`), имя сцены из `keyfill.config.ts` или каталог со `scene.cues.ts` / `scene.tsx`. Без сценария сцена получает `play` в момент 0.

Время — одна грамматика для `--t` и `--cue`: `1.2` (секунды), `60f` (кадры), `settled` (кадр «в эфире», когда закончились все `in`; по умолчанию), `settled+0.5`, маркеры таймлайна `goalHome+0.1`, диапазон `0..2` или `ready..out` для `check`, `filmstrip` и `video`. Там, где нужен диапазон, имя без смещения — сегмент: `in` (от play до settled), маркер таймлайна (до следующего маркера), `out` без такого маркера (от stop до конца). `--cue "0 play; 1.2 next; 125f stop Scoreboard"` — команды в короткой форме: play, stop, next, с компонентом — только его экземпляры; `update` и `invoke` с параметрами — в `*.cues.ts`.

Коды выхода: 0 успех, 1 ошибки в диагностиках или упавшие тесты, 2 неверный вызов, 3 сборка, 4 исключение JS, 5 сбой keyfill.

Данные сцены объявляются в `defineScene({ props: { name: String }, defaults: { name: 'Анна' }, root: (p) => ... })` и задаются из CLI: `--props data.json` или `--set name="Иван Петров"` (тип приводится по схеме, `prop<T>()` — JSON). В сценарии их меняет `at(t, 'update', { name: '...' })` без компонента. Ошибка в данных — код 2 и подсказка `keyfill schema`.

`--json` у любой команды печатает один объект `{ "keyfill": 1, command, ok, exit, entry, config, at, result, logs, next, repro }`; при ошибке — `{ ..., ok: false, error: { kind, message, diagnostics } }`. Текстовый вывод строится из той же структуры. `next` — готовые команды следующего шага (`keyfill inspect` узла с ошибкой, `keyfill explain` её правила, `keyfill test … --update`), `repro` — строка, которая воспроизводит вызов. Ошибки сборки приходят как диагностики `build.*` с местом и строкой кода, исключения JS — как `js.exception` со стеком, переведённым через sourcemap на строки TSX.

```bash
cargo run -q -p keyfill-cli -- filmstrip examples/scoreboard/scene.cues.ts --n 16 -o out/scoreboard.strip.png
```

Общие флаги: `--json`, `--props`/`--set`, `--mode dev|air` (dev — нарушение `must` это ошибка, air — применяются шаги `else`), `--bg #RRGGBB` (подложка: рисуется и учитывается в контрасте), `--seed N`, `--now MS`.

`keyfill.config.ts` (ищется вверх от каталога запуска) задаёт значения по умолчанию: `mode`, `bg`, `seed`, `now`, каталог вывода `out` и именованные сцены (`keyfill render lower`); флаги его перекрывают. Конфиг этого репозитория пишет выводы в `out/` с путём сцены внутри.

Бандл сцены кэшируется в `.keyfill/cache` каталога запуска по содержимому всех входных файлов: повторный запуск не вызывает node. `KEYFILL_CACHE_DIR` — другой каталог, `KEYFILL_NO_CACHE=1` — без кэша.

## Тесты сцен

```ts
// examples/lower-third/lower-third.keyfill.test.ts
test('в эфире', { scene, bg: '#101820', scale: 0.5, expect: { snapshot: ['dump', 'png'] } });
test('длинное имя', { scene, props: { name: '…' }, expect: { snapshot: 'dump' } });
test('сценарий', { scene: script, t: ['0..5', 2.6, 3.6], expect: { snapshot: 'dump' } });
```

```bash
cargo run -q -p keyfill-cli -- test
```

Снапшоты лежат в `__snapshots__` рядом с тестом и хранятся в git; после намеренной правки — `keyfill test <файл> --update`. Подробно — `keyfill docs test`.

## Проверки

```bash
cargo test --workspace
```

```bash
npm run typecheck
```

Обновить золотой дамп: `KEYFILL_UPDATE_SNAPSHOTS=1 cargo test -p keyfill-cli`.

## Устройство

| Каталог | Что внутри |
|---|---|
| `packages/core` | `@keyfill/core` для JS: сигналы, JSX-рантайм, примитивы, мост команд; `bundle.mjs` собирает сцену (шрифты — через import) |
| `crates/keyfill-js` | QuickJS-хост: исполняет бандл, подменяет часы и случайность |
| `crates/keyfill-core` | Дерево сцены, `look` с provenance, Taffy, текст (parley), движение (`motion.rs`), display list, дамп и inspect |
| `crates/keyfill-render` | CPU-бэкенд: исполняет display list через vello_cpu, PNG |
| `crates/keyfill-cli` | Сессия (JS-хост, сценарий, часы), слой операций (`ops.rs`), вывод и JSON-конверт (`output.rs`), сборка с кэшем и sourcemap, конфиг, тесты сцен (`suite.rs`), `doctor` и кэш, `new` (`scaffold.rs`), раскадровка, видео, просмотрщик и команды `keyfill`; справка `explain` и `docs` — в `help/`, шаблоны `keyfill new` — в `templates/` |
| `scripts/npm.mjs` | Сборка npm-пакетов `@keyfill/core`, `@keyfill/cli`, `@keyfill/cli-<платформа>` и проверка установки; `.github/workflows/release.yml` — то же под все платформы |
| `spec` | Типы публичного API и контракт-тест для tsc |
| `examples/lower-third` | Плашка: компонент, presence, transition, тема, сцена, сценарий `scene.cues.ts`, тесты `lower-third.keyfill.test.ts`, золотые дампы |
| `examples/scoreboard` | Табло: grid, stack, Span, `fit`/`must`/`else`, таймлайн с hold, циклом и сегментами, команда `goal`, эфирный сценарий, сцена с переполнением, тесты `scoreboard.keyfill.test.ts` |
| `assets/fonts` | Шрифты для примеров и тестов (Inter, OFL) |

## Лицензия

keyfill распространяется под [MIT](LICENSE-MIT) или [Apache-2.0](LICENSE-APACHE) на ваш выбор. Шрифт Inter в `assets/fonts/inter` — под [SIL Open Font License](assets/fonts/inter/OFL.txt).
