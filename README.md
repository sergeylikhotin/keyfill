# kine

Нативный движок рендера интерфейсов и эфирной графики: сцена описывается в TSX, kine рисует её без браузера. Основной автор сцен — AI-агент, поэтому движок отдаёт не только картинку, но и структуру: дамп сцены, происхождение каждого значения, диагностики с местом в исходнике.

Замысел, принципы и план — в [DESIGN.md](DESIGN.md). Контракт API — [spec/kine.d.ts](spec/kine.d.ts).

## Быстрый старт

Нужны Rust (stable, MSVC на Windows) и Node.

```bash
npm install
cargo build
```

```bash
cargo run -q -p kine-cli -- render examples/lower-third/scene.tsx -o out/lower-third.png --bg "#2b3340" --dump
```

Видео и просмотрщик (рендер видео лучше собирать в release):

```bash
cargo run -q --release -p kine-cli -- video examples/scoreboard/scene.cues.ts -o out/scoreboard.webm
```

```bash
cargo run -q --release -p kine-cli -- view examples/scoreboard/scene.cues.ts
```

## Команды

| Команда | Что делает |
|---|---|
| `kine render <entry> [-o out.png] [--scale N] [--dump]` | PNG кадра; без `--bg` — с прозрачностью |
| `kine dump <entry>` | Дерево сцены: адреса узлов, геометрия, стили, фазы движения, диагностики |
| `kine inspect <entry> <path>` | Один узел: значения, токены и слои, откуда они пришли, каналы движения, исходник |
| `kine check <entry> [--json]` | Только диагностики; код выхода 1 при ошибках |
| `kine filmstrip <entry> [--t a..b] [-o strip.png] [--n 8] [--cols 4] [--tile 480] [--dump]` | Раскадровка: n моментов диапазона одной картинкой, с `--dump` — дамп каждого |
| `kine video <entry> [--t a..b] [-o out.mov] [--codec prores\|qtrle\|vp9\|h264\|png] [--scale N]` | Видео через ffmpeg: `.mov` — ProRes 4444 с альфой, `.webm` — VP9 с альфой, `.mp4` — H.264 на подложке, каталог — PNG; побайтно воспроизводимо |
| `kine view [entry] [--dir out] [--port 4747]` | Веб-просмотрщик на localhost: галерея всех выводов с автообновлением и живая сцена с ползунком времени, дампом, событиями и inspect по клику |

`<entry>` — сцена (`defineScene`) или сценарий `*.cues.ts` (`cues(scene, ({ at }) => [...])`). Без сценария сцена получает `play` в момент 0.

Время — одна грамматика для `--t` и `--cue`: `1.2` (секунды), `60f` (кадры), `settled` (кадр «в эфире», когда закончились все `in`; по умолчанию), `settled+0.5`, маркеры таймлайна `goalHome+0.1`, диапазон `0..2` или `ready..out` для `filmstrip`. `--cue "0 play; 1.2 next; 125f stop Scoreboard"` — команды в короткой форме: play, stop, next, с компонентом — только его экземпляры; `update` и `invoke` с параметрами — в `*.cues.ts`.

Коды выхода: 0 успех, 1 ошибки в диагностиках, 2 неверный вызов, 3 сборка, 4 исключение JS, 5 сбой kine.

```bash
cargo run -q -p kine-cli -- filmstrip examples/scoreboard/scene.cues.ts --n 16 -o out/scoreboard.strip.png
```

Общие флаги: `--mode dev|air` (dev — нарушение `must` это ошибка, air — применяются шаги `else`), `--bg #RRGGBB` (подложка: рисуется и учитывается в контрасте), `--seed N`, `--now MS`.

## Проверки

```bash
cargo test --workspace
```

```bash
npm run typecheck
```

Обновить золотой дамп: `KINE_UPDATE_SNAPSHOTS=1 cargo test -p kine-cli`.

## Устройство

| Каталог | Что внутри |
|---|---|
| `packages/core` | `@kine/core` для JS: сигналы, JSX-рантайм, примитивы, мост команд; `bundle.mjs` собирает сцену (шрифты — через import) |
| `crates/kine-js` | QuickJS-хост: исполняет бандл, подменяет часы и случайность |
| `crates/kine-core` | Дерево сцены, `look` с provenance, Taffy, текст (parley), движение (`motion.rs`), display list, дамп и inspect |
| `crates/kine-render` | CPU-бэкенд: исполняет display list через vello_cpu, PNG |
| `crates/kine-cli` | Сессия (JS-хост, сценарий, часы), раскадровка и команды `kine` |
| `spec` | Типы публичного API и контракт-тест для tsc |
| `examples/lower-third` | Плашка: компонент, presence, transition, тема, сцена, сценарий `scene.cues.ts`, золотые дампы |
| `examples/scoreboard` | Табло: grid, stack, Span, `fit`/`must`/`else`, таймлайн с hold, циклом и сегментами, команда `goal`, эфирный сценарий, сцена с переполнением |
| `assets/fonts` | Шрифты для примеров и тестов (Inter, OFL) |
