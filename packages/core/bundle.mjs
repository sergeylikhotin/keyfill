// Бандлер сцены для kine: `node bundle.mjs <scene.tsx>` печатает в stdout JSON
// `{ code, map, inputs }` — IIFE-бандл, его sourcemap и все входные файлы (для кэша kine).
// Точка входа — сцена, сценарий, файл тестов или kine.config.ts; что с ней делать, решает
// `__kine_main` из `@kine/core`.
// При ошибке печатает `{ errors: [{ file, line, column, text, lineText }] }` и выходит с кодом 1.
//
// JSX собирается в dev-режиме (у элементов есть место в исходнике), `@kine/core` указывает
// на реализацию рядом с этим файлом, а импорт шрифта (`import inter from './Inter.otf'`)
// превращается в абсолютный путь к файлу: путь разрешает бандлер, как у любого импорта,
// а ядро читает файл по этому пути.

import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const [entry] = process.argv.slice(2);
if (!entry) {
  console.error('usage: node bundle.mjs <scene.tsx>');
  process.exit(2);
}

const core = path.join(path.dirname(fileURLToPath(import.meta.url)), 'src');

const fonts = {
  name: 'kine-fonts',
  setup(b) {
    b.onLoad({ filter: /\.(otf|ttf|woff2?)$/i }, (args) => ({
      contents: `export default ${JSON.stringify(args.path)};`,
      loader: 'js',
    }));
  },
};

const cwd = process.cwd();
const slash = (p) => p.replaceAll('\\', '/');
const entryPath = slash(path.resolve(entry));
// Пути в выводе — относительно каталога запуска, как места в dev-JSX; вне него — абсолютные.
const shown = (abs) => {
  const rel = path.relative(cwd, abs);
  return slash(rel.startsWith('..') || path.isAbsolute(rel) ? abs : rel);
};

try {
  const result = await build({
    stdin: {
      // У файла тестов нет экспорта по умолчанию: берём модуль целиком.
      contents: `import * as m from ${JSON.stringify(entryPath)};\nimport { __kine_main } from '@kine/core';\n__kine_main(m.default);\n`,
      resolveDir: cwd,
      sourcefile: 'kine-entry.ts',
      loader: 'ts',
    },
    bundle: true,
    write: false,
    // Файл не пишется; outfile задаёт только базу для путей в sourcemap.
    outfile: path.join(cwd, 'kine-bundle.js'),
    sourcemap: 'external',
    sourcesContent: false,
    metafile: true,
    format: 'iife',
    platform: 'neutral',
    target: 'es2020',
    jsx: 'automatic',
    jsxDev: true,
    jsxImportSource: '@kine/core',
    alias: { '@kine/core': core },
    plugins: [fonts],
    logLevel: 'silent',
  });
  const code = result.outputFiles.find((f) => f.path.endsWith('.js')).text;
  const map = JSON.parse(result.outputFiles.find((f) => f.path.endsWith('.map')).text);
  map.sources = map.sources.map((s) => shown(path.resolve(cwd, s)));
  const inputs = Object.keys(result.metafile.inputs)
    .filter((p) => p !== '<stdin>')
    .map((p) => slash(path.resolve(cwd, p)));
  process.stdout.write(JSON.stringify({ code, map, inputs }));
} catch (e) {
  // У ошибок esbuild есть файл, строка и фрагмент кода — отдаём их структурой.
  const errors = (e.errors ?? [{ text: String(e) }]).map((err) => ({
    file: err.location ? shown(path.resolve(cwd, err.location.file)) : undefined,
    line: err.location?.line,
    column: err.location ? err.location.column + 1 : undefined,
    text: err.text,
    lineText: err.location?.lineText,
  }));
  process.stdout.write(JSON.stringify({ errors }));
  process.exit(1);
}
