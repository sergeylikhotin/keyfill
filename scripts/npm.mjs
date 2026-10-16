// Сборка npm-пакетов kine из репозитория:
//
//   node scripts/npm.mjs [--target win32-x64] [--binary target/release/kine.exe] [--out dist/npm] [--pack] [--smoke]
//
// - @kine/core            — типы (из spec/kine.d.ts в виде модулей), рантайм src, bundle.mjs; зависит от esbuild;
// - @kine/cli             — команда `kine` (bin/kine.js): запускает бинарь платформы с KINE_CORE;
// - @kine/cli-<платформа> — нативный бинарь; ставится через optionalDependencies по os и cpu.
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

/** Платформы, под которые есть пакет бинаря: имя пакета — `@kine/cli-<платформа>`. */
const TARGETS = ['win32-x64', 'linux-x64', 'darwin-arm64', 'darwin-x64'];
const target = value('--target', `${process.platform}-${process.arch}`);
if (!TARGETS.includes(target)) throw new Error(`--target: ${target}; есть ${TARGETS.join(', ')}`);
const [targetOs, targetCpu] = target.split('-');
const exe = targetOs === 'win32' ? 'kine.exe' : 'kine';
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
// repository и homepage добавить, когда у репозитория появится публичный адрес.
const common = { version, license };

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
  const spec = fs.readFileSync(path.join(root, 'spec', 'kine.d.ts'), 'utf8');
  const mods = modules(spec);
  const get = (name) => mods.find((m) => m.name === name) ?? (() => { throw new Error(`в спеке нет модуля ${name}`); })();
  const head = `// Типы @kine/core ${version}. Сгенерировано из spec/kine.d.ts репозитория kine.\n`;
  write(path.join(dir, 'index.d.ts'), `${head}/// <reference path="./fonts.d.ts" />\n${asModule(get('@kine/core').body)}\n`);
  write(path.join(dir, 'jsx-runtime.d.ts'), `${head}${asModule(get('@kine/core/jsx-runtime').body)}\n`);
  write(path.join(dir, 'jsx-dev-runtime.d.ts'), `${head}${asModule(get('@kine/core/jsx-dev-runtime').body)}\n`);
  // Импорты шрифтов — глобальные объявления: остаются ambient.
  const fonts = mods.filter((m) => m.name.startsWith('*.')).map((m) => m.raw.join('\n'));
  write(path.join(dir, 'fonts.d.ts'), `${head}// Импорт шрифта даёт FontFile: бандлер kine подставляет абсолютный путь к файлу.\n${fonts.join('\n')}\n`);
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
      name: '@kine/core',
      ...common,
      description: 'kine: API сцен (TSX), рантайм и бандлер. Команда kine — в @kine/cli.',
      type: 'module',
      types: './index.d.ts',
      exports: {
        '.': entry('index'),
        './jsx-runtime': entry('jsx-runtime'),
        './jsx-dev-runtime': entry('jsx-dev-runtime'),
        './package.json': './package.json',
      },
      files: ['src', 'bundle.mjs', '*.d.ts'],
      dependencies: { esbuild },
    }),
  );
  write(path.join(dir, 'README.md'), `# @kine/core\n\nAPI сцен kine: \`defineScene\`, \`component\`, примитивы, движение, \`test\`, \`defineConfig\`. Ставится вместе с командой: \`npm i -D @kine/cli\`. Справка — \`npx kine docs\`.\n`);
  return dir;
}

function cli() {
  const dir = path.join(out, 'cli');
  fs.rmSync(dir, { recursive: true, force: true });
  write(path.join(dir, 'bin', 'kine.js'), SHIM);
  write(
    path.join(dir, 'package.json'),
    json({
      name: '@kine/cli',
      ...common,
      description: 'kine: рендер TSX-сцен эфирной графики и интерфейсов без браузера.',
      bin: { kine: 'bin/kine.js' },
      files: ['bin'],
      dependencies: { '@kine/core': version },
      optionalDependencies: Object.fromEntries(TARGETS.map((t) => [`@kine/cli-${t}`, version])),
    }),
  );
  write(path.join(dir, 'README.md'), `# @kine/cli\n\n\`npm i -D @kine/cli\`, затем \`npx kine new lower-third\`, \`npx kine check lower-third\`, \`npx kine test\`. Окружение — \`npx kine doctor\`, справка — \`npx kine docs\`.\n`);
  return dir;
}

function platform() {
  if (!fs.existsSync(binary)) throw new Error(`нет бинаря ${binary}: cargo build --release -p kine-cli`);
  const dir = path.join(out, `cli-${target}`);
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(path.join(dir, 'bin'), { recursive: true });
  fs.copyFileSync(binary, path.join(dir, 'bin', exe));
  if (targetOs !== 'win32') fs.chmodSync(path.join(dir, 'bin', exe), 0o755);
  write(
    path.join(dir, 'package.json'),
    json({ name: `@kine/cli-${target}`, ...common, description: `Бинарь kine для ${target}; ставится через @kine/cli.`, os: [targetOs], cpu: [targetCpu], files: ['bin'] }),
  );
  return dir;
}

const SHIM = `#!/usr/bin/env node
// Команда kine: запускает нативный бинарь из пакета платформы. KINE_CORE говорит бинарю,
// где @kine/core (бандлер и рантайм сцен) — рядом в node_modules.
'use strict';
const { spawnSync } = require('node:child_process');
const path = require('node:path');

const target = process.platform + '-' + process.arch;
const pkg = '@kine/cli-' + target;
let bin;
try {
  bin = require.resolve(pkg + '/bin/kine' + (process.platform === 'win32' ? '.exe' : ''));
} catch {
  console.error('kine: нет бинаря для ' + target + ' (пакет ' + pkg + '). Платформы: ${TARGETS.join(', ')}. ' +
    'Переустанови зависимости без --no-optional / --omit=optional.');
  process.exit(5);
}
const core = path.dirname(require.resolve('@kine/core/package.json'));
const r = spawnSync(bin, process.argv.slice(2), { stdio: 'inherit', env: { ...process.env, KINE_CORE: process.env.KINE_CORE || core } });
if (r.error) {
  console.error('kine: ' + r.error.message);
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
  const project = fs.mkdtempSync(path.join(os.tmpdir(), 'kine-smoke-'));
  console.log(`smoke: ${project}`);
  write(path.join(project, 'package.json'), json({ name: 'kine-smoke', private: true }));
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
  step('kine doctor', ['kine', 'doctor']);
  step('kine new lower-third', ['kine', 'new', 'lower-third']);
  step('kine check lower-third', ['kine', 'check', 'lower-third']);
  step('kine test', ['kine', 'test']);
  step('kine render lower-third', ['kine', 'render', 'lower-third']);
  step('tsc', ['tsc', '-p', '.']);
  return project;
}

fs.mkdirSync(out, { recursive: true });
const dirs = [core(), cli(), platform()];
console.log(`kine ${version}: ${dirs.map((d) => path.relative(root, d)).join(', ')}`);
if (flag('--pack') || flag('--smoke')) {
  const tarballs = dirs.map(pack);
  console.log(tarballs.map((t) => `  ${path.relative(root, t)}`).join('\n'));
  if (flag('--smoke')) smoke(tarballs);
}
