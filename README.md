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
cargo run -q -p kine-cli -- render examples/lower-third/scene.tsx -o out/lower-third.png --bg #2b3340 --dump
```

## Команды

| Команда | Что делает |
|---|---|
| `kine render <scene.tsx> [-o out.png] [--scale N] [--bg #RRGGBB] [--dump]` | PNG кадра; без `--bg` — с прозрачностью |
| `kine dump <scene.tsx>` | Дерево сцены: адреса узлов, геометрия, стили, диагностики |
| `kine inspect <scene.tsx> <path>` | Один узел: значения, токены и слои, откуда они пришли, исходник |
| `kine check <scene.tsx> [--json]` | Только диагностики; код выхода 1 при ошибках |

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
| `packages/core` | `@kine/core` для JS: сигналы, JSX-рантайм, примитивы, мост команд |
| `crates/kine-js` | QuickJS-хост: исполняет бандл, подменяет часы и случайность |
| `crates/kine-core` | Дерево сцены, `look` с provenance, Taffy, текст (parley), дамп и inspect |
| `crates/kine-render` | Рендер кадра через vello_cpu, PNG |
| `crates/kine-cli` | Конвейер TSX → кадр и команды `kine` |
| `spec` | Типы публичного API и контракт-тест для tsc |
| `examples/lower-third` | Эталонная плашка: компонент, таймлайн, тема, сцена, золотой дамп |
