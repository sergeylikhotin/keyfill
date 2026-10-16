# Changelog

All notable changes to keyfill are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/): before 1.0, a breaking change bumps the minor
version. The `--json` envelope has its own schema version (`"keyfill": 1`), independent of the
package version.

## [Unreleased]

### Added

- Prop types described at runtime: `prop.string`, `prop.number({ min, max, int })`, `prop.boolean`,
  `prop.enum([...])`, `prop.list`, `prop.object`, `prop.image`, `prop.json<T>()`, each with
  `default` and `description`. TypeScript types are inferred from them, props with `default` are
  optional, `keyfill schema --json` gives a JSON Schema for every prop and command parameter, and
  values from `--props`/`--set`, cues and channel commands are checked with a path in the error
  (`players[1].number`).
- Channels: `defineChannel({ stage, fps, components })` is a catalog of components that commands
  put on numbered layers (`load`, `play`, `next`, `stop`, `update`, `invoke`, `clear`). An instance
  leaving a layer plays its `out` and is then removed.
- Channel journals (`*.journal.jsonl`): batches of commands by channel frame, applied atomically.
  A journal is an ordinary `<entry>` for `render`, `dump`, `check`, `filmstrip`, `video`, `view` and
  `schema`, and replays to the same frames as the live session that wrote it. New diagnostics
  `channel.rejected` and `journal.diverged`; new help topics `keyfill docs props` and `keyfill docs channel`.

- Lists from data: `<For each={...} by={(item) => item.id}>{(item, index) => ...}</For>` inside a `Box`.
  The same key keeps its nodes and animates new values, a new key mounts and plays `in` (its own or
  the container's `stagger`), a missing key plays `out` in place and is removed afterwards; only moved
  items are reordered. Items are addressed by key in dumps (`box[key=zen]`). New diagnostic
  `list.duplicate-key`, help topic `keyfill docs list`, example `examples/standings`.

### Changed

- **Breaking:** `prop<T>()` is replaced by `prop.json<T>()`.

## [0.1.0-alpha.1] - 2026-10-16

First public prerelease. Linux and macOS binaries are built by CI and have not been tested by hand yet.

### Added

- Scenes in TSX: `defineScene` with typed props and defaults, components with parts and commands,
  signals, `Box`, `Text`, `Span`; `look` layers with provenance and themes; row, column, grid and
  stack layout, anchors to safe areas and parts; text `fit`.
- Assertions: `must` (`fit`, `inside`, `contrast`, `visible`) with independent `else` fallbacks,
  errors in dev mode and automatic fallbacks in air mode.
- Motion computed in closed form: `transition`, `presence` (in, loop, out), springs, `stagger`,
  timelines with markers, holds, loops, segments and events, animated `count()` and `clock()`;
  scripts of operator cues (`*.cues.ts`, `--cue`).
- CLI as a contract for agents: `render`, `dump`, `inspect`, `check` over ranges and segments,
  `filmstrip`, `video` (ProRes 4444, QuickTime Animation, VP9 with alpha, H.264, PNG frames;
  byte-reproducible), `view`, `schema`, `--json` envelope with `next` and `repro`, exit codes,
  build and JS diagnostics mapped to TSX through source maps, bundle cache.
- `keyfill test` with dump and pixel snapshots, `keyfill perf`, `keyfill explain` and `keyfill docs` built
  into the binary, `keyfill.config.ts`.
- `keyfill new` templates, `keyfill doctor`, `keyfill cache`; npm packages `@keyfill/cli`, `@keyfill/core` and
  native binaries for win32-x64, linux-x64 (glibc 2.35+, libfontconfig) and darwin-arm64. Intel Macs (darwin-x64) build from
  source with `cargo build --release`.

[Unreleased]: https://github.com/sergeylikhotin/keyfill/compare/v0.1.0-alpha.1...HEAD
[0.1.0-alpha.1]: https://github.com/sergeylikhotin/keyfill/releases/tag/v0.1.0-alpha.1
