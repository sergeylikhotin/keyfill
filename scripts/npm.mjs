// Сборка npm-пакетов keyfill из репозитория:
//
//   node scripts/npm.mjs [--target win32-x64] [--binary target/release/keyfill.exe] [--out dist/npm] [--pack] [--smoke]
//
// - @keyfill/core            — типы (из spec/keyfill.d.ts в виде модулей), рантайм src, bundle.mjs; зависит от esbuild;
// - @keyfill/cli             — команда `keyfill` (bin/keyfill.js): запускает бинарь платформы с KEYFILL_CORE;
// - @keyfill/cli-<платформа> — нативный бинарь; ставится через optionalDependencies по os и cpu.
//
// --pack упаковывает каждый пакет в .tgz (npm pack), --smoke ставит их в чистый временный проект
// и проходит путь пользователя: doctor, new, check, test, tsc. Публикация — `npm publish` каждого
// .tgz, этот скрипт её не делает.

import { execSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const value = (name, fallback) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : fallback;
};

/** Платформы, под которые есть пакет бинаря: имя пакета — `@keyfill/cli-<платформа>`. */
const TARGETS = ['win32-x64', 'linux-x64', 'darwin-arm64', 'darwin-x64'];
const target = value('--target', `${process.platform}-${process.arch}`);
if (!TARGETS.includes(target)) throw new Error(`--target: ${target}; есть ${TARGETS.join(', ')}`);
const [targetOs, targetCpu] = target.split('-');
const exe = targetOs === 'win32' ? 'keyfill.exe' : 'keyfill';
const binary = path.resolve(root, value('--binary', path.join('target', 'release', exe)));
const out = path.resolve(root, value('--out', path.join('dist', 'npm')));

const cargo = fs.readFileSync(path.join(root, 'Cargo.toml'), 'utf8');
const version = /\[workspace\.package\][^[]*?version\s*=\s*"([^"]+)"/s.exec(cargo)[1];
const license = /\[workspace\.package\][^[]*?license\s*=\s*"([^"]+)"/s.exec(cargo)[1];
const esbuild = JSON.parse(fs.readFileSync(path.join(root, 'package.json'), 'utf8')).devDependencies.esbuild;

const write = (file, text) => {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, text);
};
const json = (v) => `${JSON.stringify(v, null, 2)}\n`;
const repo = 'https://github.com/sergeylikhotin/keyfill';
const common = {
  version,
  license,
  homepage: repo,
  repository: { type: 'git', url: `git+${repo}.git` },
  bugs: { url: `${repo}/issues` },
  keywords: ['keyfill', 'broadcast', 'graphics', 'tsx', 'render', 'motion', 'overlay', 'ai-agent'],
};

/** README пакета и лицензии репозитория. */
function docs(dir, lines) {
  write(path.join(dir, 'README.md'), `${[...lines, '', 'License: MIT OR Apache-2.0.'].join('\n')}\n`);
  for (const f of ['LICENSE-MIT', 'LICENSE-APACHE']) fs.copyFileSync(path.join(root, f), path.join(dir, f));
}

// ─── Типы: ambient-модули спеки → файлы-модули ──────────────────────────────

/** Блоки `declare module 'имя' { … }` спеки: имя и тело без внешнего отступа. */
function modules(spec) {
  const lines = spec.split(/\r?\n/);
  const found = [];
  for (let i = 0; i < lines.length; i++) {
    const m = /^declare module '([^']+)' \{$/.exec(lines[i]);
    if (!m) continue;
    const body = [];
    for (i++; lines[i] !== '}'; i++) body.push(lines[i].replace(/^ {2}/, ''));
    found.push({ name: m[1], body, raw: [`declare module '${m[1]}' {`, ...lines.slice(i - body.length, i), '}'] });
  }
  return found;
}

/** Тело ambient-модуля как файл .d.ts: объявления верхнего уровня без export получают `declare`. */
function asModule(body) {
  return body.map((l) => (/^(const|function|let|var|class|namespace|enum) /.test(l) ? `declare ${l}` : l)).join('\n');
}

function types(dir) {
  const spec = fs.readFileSync(path.join(root, 'spec', 'keyfill.d.ts'), 'utf8');
  const mods = modules(spec);
  const get = (name) => mods.find((m) => m.name === name) ?? (() => { throw new Error(`в спеке нет модуля ${name}`); })();
  const head = `// Типы @keyfill/core ${version}. Сгенерировано из spec/keyfill.d.ts репозитория keyfill.\n`;
  write(path.join(dir, 'index.d.ts'), `${head}/// <reference path="./fonts.d.ts" />\n${asModule(get('@keyfill/core').body)}\n`);
  write(path.join(dir, 'jsx-runtime.d.ts'), `${head}${asModule(get('@keyfill/core/jsx-runtime').body)}\n`);
  write(path.join(dir, 'jsx-dev-runtime.d.ts'), `${head}${asModule(get('@keyfill/core/jsx-dev-runtime').body)}\n`);
  // Импорты шрифтов — глобальные объявления: остаются ambient.
  const fonts = mods.filter((m) => m.name.startsWith('*.')).map((m) => m.raw.join('\n'));
  write(path.join(dir, 'fonts.d.ts'), `${head}// Импорт шрифта даёт FontFile: бандлер keyfill подставляет абсолютный путь к файлу.\n${fonts.join('\n')}\n`);
}

// ─── Пакеты ─────────────────────────────────────────────────────────────────

function core() {
  const dir = path.join(out, 'core');
  fs.rmSync(dir, { recursive: true, force: true });
  const src = path.join(root, 'packages', 'core', 'src');
  fs.mkdirSync(path.join(dir, 'src'), { recursive: true });
  // conformance.ts сверяет реализацию со спекой и в пакет не идёт.
  for (const f of fs.readdirSync(src)) if (f !== 'conformance.ts') fs.copyFileSync(path.join(src, f), path.join(dir, 'src', f));
  fs.copyFileSync(path.join(root, 'packages', 'core', 'bundle.mjs'), path.join(dir, 'bundle.mjs'));
  types(dir);
  const entry = (name) => ({ types: `./${name}.d.ts`, default: `./src/${name}.ts` });
  write(
    path.join(dir, 'package.json'),
    json({
      name: '@keyfill/core',
      ...common,
      description: 'keyfill scene API for TSX: components, look, layout, motion, tests, config. The keyfill command is in @keyfill/cli.',
      type: 'module',
      types: './index.d.ts',
      exports: {
        '.': entry('index'),
        './jsx-runtime': entry('jsx-runtime'),
        './jsx-dev-runtime': entry('jsx-dev-runtime'),
        './package.json': './package.json',
      },
      files: ['src', 'bundle.mjs', '*.d.ts', 'LICENSE-*'],
      dependencies: { esbuild },
    }),
  );
  docs(dir, [
    '# @keyfill/core',
    '',
    `Scene API for [keyfill](${repo}): \`defineScene\`, \`component\`, primitives, \`look\`, layout, motion, \`test\`, \`defineConfig\`,`,
    'plus the runtime and the bundler keyfill runs scenes with.',
    '',
    'Install it together with the command: `npm i -D @keyfill/cli`. Built-in reference: `npx keyfill docs`.',
  ]);
  return dir;
}

function cli() {
  const dir = path.join(out, 'cli');
  fs.rmSync(dir, { recursive: true, force: true });
  write(path.join(dir, 'bin', 'keyfill.js'), SHIM);
  write(
    path.join(dir, 'package.json'),
    json({
      name: '@keyfill/cli',
      ...common,
      description: 'keyfill: render broadcast graphics and UI from TSX without a browser, built for AI agents.',
      bin: { keyfill: 'bin/keyfill.js' },
      files: ['bin', 'LICENSE-*'],
      dependencies: { '@keyfill/core': version },
      optionalDependencies: Object.fromEntries(TARGETS.map((t) => [`@keyfill/cli-${t}`, version])),
    }),
  );
  docs(dir, [
    '# @keyfill/cli',
    '',
    `The \`keyfill\` command: renders TSX scenes of broadcast graphics and interfaces without a browser. See [keyfill](${repo}).`,
    '',
    '```bash',
    'npm i -D @keyfill/cli typescript',
    'npx keyfill new lower-third',
    'npx keyfill check lower-third',
    'npx keyfill test',
    '```',
    '',
    'Environment check: `npx keyfill doctor`. Reference: `npx keyfill docs`. The native binary comes from a platform package',
    `(${TARGETS.map((t) => `\`@keyfill/cli-${t}\``).join(', ')}), picked by npm for your OS and CPU.`,
  ]);
  return dir;
}

/**
 * Лицензии крейтов, вкомпилированных в бинарь: MIT, Apache-2.0 и остальные требуют поставлять
 * их тексты вместе с бинарём. Список — `cargo tree` для keyfill-cli под платформу сборки, тексты —
 * файлы LICENSE / COPYING / NOTICE из исходников крейтов (и QuickJS внутри rquickjs-sys).
 */
function thirdParty() {
  const tree = execSync('cargo tree -p keyfill-cli -e normal --prefix none --format "{p}"', { cwd: root, encoding: 'utf8' });
  const used = new Set(tree.split(/\r?\n/).filter((l) => l && !l.includes('(')).map((l) => l.replace(/ \(\*\)$/, '').trim()));
  const meta = JSON.parse(execSync('cargo metadata --format-version 1', { cwd: root, encoding: 'utf8', maxBuffer: 64 << 20 }));
  const isText = (f) => /^(licen[cs]e|copying|notice)/i.test(f);
  const crates = meta.packages.filter((p) => used.has(`${p.name} v${p.version}`)).sort((a, b) => a.name.localeCompare(b.name));
  // Одинаковые тексты (Apache-2.0 у десятков крейтов) — один раз, со списком крейтов.
  const texts = new Map();
  const bare = [];
  for (const p of crates) {
    const dir = path.dirname(p.manifest_path);
    const files = fs.readdirSync(dir).filter(isText).map((f) => path.join(dir, f));
    for (const sub of ['quickjs', 'quickjs-ng']) {
      const d = path.join(dir, sub);
      if (fs.existsSync(d)) files.push(...fs.readdirSync(d).filter(isText).map((f) => path.join(d, f)));
    }
    if (!files.length) bare.push(p);
    for (const f of files) {
      const text = fs.readFileSync(f, 'utf8').replace(/\r\n/g, '\n').trim();
      const key = text.replace(/\s+/g, ' ');
      if (!texts.has(key)) texts.set(key, { text, users: [] });
      texts.get(key).users.push(`${p.name} ${p.version}${f.includes('quickjs') ? ' (QuickJS)' : ''}`);
    }
  }
  const out = [`Third-party software in the keyfill ${version} binary (${target})`, ''];
  for (const p of crates) out.push(`${p.name} ${p.version} — ${p.license ?? 'see repository'}${p.repository ? ` — ${p.repository}` : ''}`);
  if (bare.length) {
    out.push('', `Crates without a license file (texts of their SPDX licenses: LICENSE-MIT, LICENSE-APACHE in this package): ${bare.map((p) => p.name).join(', ')}`);
  }
  for (const { text, users } of texts.values()) out.push('', '='.repeat(78), `Used by: ${users.join(', ')}`, '', text);
  return `${out.join('\n')}\n`;
}

function platform() {
  if (!fs.existsSync(binary)) throw new Error(`нет бинаря ${binary}: cargo build --release -p keyfill-cli`);
  const dir = path.join(out, `cli-${target}`);
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(path.join(dir, 'bin'), { recursive: true });
  fs.copyFileSync(binary, path.join(dir, 'bin', exe));
  if (targetOs !== 'win32') fs.chmodSync(path.join(dir, 'bin', exe), 0o755);
  write(
    path.join(dir, 'package.json'),
    json({ name: `@keyfill/cli-${target}`, ...common, description: `keyfill native binary for ${target}; installed by @keyfill/cli.`, os: [targetOs], cpu: [targetCpu], files: ['bin', 'LICENSE-*', 'THIRD-PARTY-LICENSES.txt'] }),
  );
  write(path.join(dir, 'THIRD-PARTY-LICENSES.txt'), thirdParty());
  docs(dir, [`# @keyfill/cli-${target}`, '', `Native [keyfill](${repo}) binary for ${target}. Install \`@keyfill/cli\` instead: it picks this package for your platform.`]);
  return dir;
}

const SHIM = `#!/usr/bin/env node
// Команда keyfill: запускает нативный бинарь из пакета платформы. KEYFILL_CORE говорит бинарю,
// где @keyfill/core (бандлер и рантайм сцен) — рядом в node_modules.
'use strict';
const { spawnSync } = require('node:child_process');
const path = require('node:path');

const target = process.platform + '-' + process.arch;
const pkg = '@keyfill/cli-' + target;
let bin;
try {
  bin = require.resolve(pkg + '/bin/keyfill' + (process.platform === 'win32' ? '.exe' : ''));
} catch {
  console.error('keyfill: нет бинаря для ' + target + ' (пакет ' + pkg + '). Платформы: ${TARGETS.join(', ')}. ' +
    'Переустанови зависимости без --no-optional / --omit=optional.');
  process.exit(5);
}
const core = path.dirname(require.resolve('@keyfill/core/package.json'));
const r = spawnSync(bin, process.argv.slice(2), { stdio: 'inherit', env: { ...process.env, KEYFILL_CORE: process.env.KEYFILL_CORE || core } });
if (r.error) {
  console.error('keyfill: ' + r.error.message);
  process.exit(5);
}
if (r.signal) process.kill(process.pid, r.signal);
process.exit(r.status ?? 5);
`;

// ─── Упаковка и проверка установки ──────────────────────────────────────────

const npm = process.platform === 'win32' ? 'npm.cmd' : 'npm';
const npx = process.platform === 'win32' ? 'npx.cmd' : 'npx';
// npm и npx на Windows — .cmd, их запускает только shell: команда собирается строкой с кавычками.
const quote = (a) => (/^[\w@%+=:,./\-]+$/.test(a) ? a : `"${a.replaceAll('"', '\\"')}"`);
const run = (cmd, argv, cwd) => execSync([cmd, ...argv].map(quote).join(' '), { cwd, stdio: ['ignore', 'pipe', 'inherit'], encoding: 'utf8' });

function pack(dir) {
  const name = run(npm, ['pack', '--pack-destination', out, '--silent'], dir).trim().split(/\r?\n/).pop();
  return path.join(out, name);
}

function smoke(tarballs) {
  const project = fs.mkdtempSync(path.join(os.tmpdir(), 'keyfill-smoke-'));
  console.log(`smoke: ${project}`);
  write(path.join(project, 'package.json'), json({ name: 'keyfill-smoke', private: true }));
  run(npm, ['install', '--save-dev', '--no-audit', '--no-fund', ...tarballs, 'typescript@5.8.3'], project);
  const step = (title, argv, okCodes = [0]) => {
    let code = 0;
    let text = '';
    try {
      text = run(npx, argv, project);
    } catch (e) {
      code = e.status;
      text = `${e.stdout ?? ''}`;
    }
    const ok = okCodes.includes(code);
    console.log(`${ok ? 'ok  ' : 'FAIL'} ${title} (код ${code})`);
    if (!ok) {
      console.log(text);
      process.exitCode = 1;
    }
    return text;
  };
  step('keyfill doctor', ['keyfill', 'doctor']);
  step('keyfill new lower-third', ['keyfill', 'new', 'lower-third']);
  step('keyfill check lower-third', ['keyfill', 'check', 'lower-third']);
  step('keyfill test', ['keyfill', 'test']);
  step('keyfill render lower-third', ['keyfill', 'render', 'lower-third']);
  step('tsc', ['tsc', '-p', '.']);
  return project;
}

fs.mkdirSync(out, { recursive: true });
const dirs = [core(), cli(), platform()];
console.log(`keyfill ${version}: ${dirs.map((d) => path.relative(root, d)).join(', ')}`);
if (flag('--pack') || flag('--smoke')) {
  // Пакеты прошлых версий не должны попасть в публикацию вместе с новыми.
  for (const f of fs.readdirSync(out)) if (f.endsWith('.tgz')) fs.rmSync(path.join(out, f));
  const tarballs = dirs.map(pack);
  console.log(tarballs.map((t) => `  ${path.relative(root, t)}`).join('\n'));
  if (flag('--smoke')) smoke(tarballs);
}
