// Бандлер сцены для kine: `node bundle.mjs <scene.tsx>` печатает IIFE-бандл в stdout.
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

const entryPath = path.resolve(entry).replaceAll('\\', '/');

try {
  const result = await build({
    stdin: {
      contents: `import scene from ${JSON.stringify(entryPath)};\nimport { mount } from '@kine/core';\nmount(scene);\n`,
      resolveDir: process.cwd(),
      sourcefile: 'kine-entry.ts',
      loader: 'ts',
    },
    bundle: true,
    write: false,
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
  process.stdout.write(result.outputFiles[0].text);
} catch (e) {
  // Ошибки esbuild уже содержат файл, строку и фрагмент кода — отдаём их как есть.
  for (const err of e.errors ?? []) {
    const loc = err.location ? `${err.location.file}:${err.location.line}:${err.location.column + 1}: ` : '';
    console.error(`${loc}${err.text}`);
    if (err.location?.lineText) console.error(`    ${err.location.lineText}`);
  }
  if (!e.errors) console.error(String(e));
  process.exit(1);
}
