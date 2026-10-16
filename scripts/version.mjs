// Версия kine — одна на всё: `version` в [workspace.package] Cargo.toml. Из неё берут версию
// крейты, бинарь (`kine --version`, кэш бандлов, проверка @kine/core в `kine doctor`) и все
// npm-пакеты (scripts/npm.mjs).
//
//   node scripts/version.mjs 0.1.0            поднять версию: Cargo.toml и Cargo.lock
//   node scripts/version.mjs --check v0.1.0   CI: тег совпадает с версией, в CHANGELOG есть раздел
//   node scripts/version.mjs --dist-tag       npm dist-tag версии: next для пререлиза, иначе latest
//
// Коммит и тег скрипт не делает — печатает команды.

import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const cargoToml = path.join(root, 'Cargo.toml');
const changelog = path.join(root, 'CHANGELOG.md');
const SEMVER = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z.-]+))?$/;
const VERSION_LINE = /(\[workspace\.package\][^[]*?version\s*=\s*")([^"]+)(")/s;

const current = () => VERSION_LINE.exec(fs.readFileSync(cargoToml, 'utf8'))[2];
/** Пререлиз (`0.1.0-alpha.1`) идёт в npm под тегом next: `npm i @kine/cli` его не поставит. */
const distTag = (version) => (SEMVER.exec(version)[4] ? 'next' : 'latest');
const hasChangelog = (version) => fs.existsSync(changelog) && fs.readFileSync(changelog, 'utf8').includes(`## [${version}]`);

function fail(message) {
  console.error(`version: ${message}`);
  process.exit(1);
}

const [arg, value] = process.argv.slice(2);
if (arg === '--check') {
  const tag = (value ?? '').replace(/^refs\/tags\//, '');
  const version = current();
  if (tag !== `v${version}`) fail(`тег ${tag || '(нет)'} не совпадает с версией v${version} в Cargo.toml`);
  if (!hasChangelog(version)) fail(`в CHANGELOG.md нет раздела ## [${version}]`);
  console.log(`v${version} (${distTag(version)})`);
} else if (arg === '--dist-tag') {
  console.log(distTag(current()));
} else if (arg && SEMVER.test(arg)) {
  const before = current();
  fs.writeFileSync(cargoToml, fs.readFileSync(cargoToml, 'utf8').replace(VERSION_LINE, `$1${arg}$3`));
  // Версии крейтов рабочего пространства в Cargo.lock.
  execFileSync('cargo', ['update', '--workspace', '--offline'], { cwd: root, stdio: 'inherit' });
  console.log(`${before} → ${arg} (npm: ${distTag(arg)})`);
  if (!hasChangelog(arg)) console.log(`добавь в CHANGELOG.md раздел ## [${arg}] — без него CI не опубликует релиз`);
  console.log(`дальше: git commit -am "release: v${arg}" && git tag v${arg}`);
} else {
  fail('ожидается версия (0.1.0, 0.1.0-alpha.2), --check <тег> или --dist-tag');
}
