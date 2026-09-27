import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { nextVersion, planVersion, publishInstaller } from './release.mjs';

async function fixture(t) {
  const directory = await mkdtemp(join(tmpdir(), 'heydb-release-test-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  await mkdir(join(directory, 'src-tauri'));
  const files = {
    'package.json': JSON.stringify({ name: 'hey-db', version: '0.1.1', scripts: { test: 'vitest run' } }),
    'package-lock.json': JSON.stringify({ version: '0.1.1', packages: { '': { version: '0.1.1' }, 'node_modules/example': { version: '0.1.1' } } }),
    'src-tauri/tauri.conf.json': JSON.stringify({ version: '0.1.1', identifier: 'app.heydb.desktop' }),
    'src-tauri/Cargo.toml': '[package]\nname = "hey-db"\nversion = "0.1.1"\n\n[dependencies]\nexample = "0.1.1"\n',
    'src-tauri/Cargo.lock': '# lockfile\nversion = 4\n\n[[package]]\nname = "example"\nversion = "0.1.1"\n\n[[package]]\nname = "hey-db"\nversion = "0.1.1"\n',
  };
  for (const [name, text] of Object.entries(files)) await writeFile(join(directory, name), text);
  return directory;
}

test('chooses the next patch, accepts explicit versions and permits a rebuild', () => {
  assert.equal(nextVersion('0.1.1'), '0.1.2');
  assert.equal(nextVersion('0.1.9'), '0.1.10');
  assert.equal(nextVersion('0.1.1', '0.1.1'), '0.1.1');
  assert.equal(nextVersion('0.1.9', '1.0.0'), '1.0.0');
});

test('rejects downgrades, nonstandard versions, and command-like inputs', () => {
  for (const version of ['0.1.0', '0.0.9', 'v0.1.2', '0.01.2', '0.1.2-beta', '--help', '0.1.2; echo invalid', '999999999999999999999.0.0']) {
    assert.throws(() => nextVersion('0.1.1', version), undefined, version);
  }
});

test('plans consistent metadata updates without changing dependency versions or writing files', async t => {
  const directory = await fixture(t);
  const plan = await planVersion(directory, '0.1.2');
  assert.equal(plan.version, '0.1.2');
  assert.equal(JSON.parse(plan.updates.get('package.json')).version, '0.1.2');
  const lock = JSON.parse(plan.updates.get('package-lock.json'));
  assert.equal(lock.version, '0.1.2');
  assert.equal(lock.packages[''].version, '0.1.2');
  assert.equal(lock.packages['node_modules/example'].version, '0.1.1');
  assert.equal(JSON.parse(plan.updates.get('src-tauri/tauri.conf.json')).version, '0.1.2');
  assert.match(plan.updates.get('src-tauri/Cargo.toml'), /name = "hey-db"\nversion = "0.1.2"/);
  assert.match(plan.updates.get('src-tauri/Cargo.toml'), /example = "0.1.1"/);
  assert.match(plan.updates.get('src-tauri/Cargo.lock'), /name = "hey-db"\nversion = "0.1.2"/);
  assert.match(plan.updates.get('src-tauri/Cargo.lock'), /name = "example"\nversion = "0.1.1"/);
  for (const [name, original] of plan.originals) assert.equal(await readFile(join(directory, name), 'utf8'), original);
});

test('refuses inconsistent version files before modifying anything', async t => {
  const directory = await fixture(t);
  await writeFile(join(directory, 'src-tauri/Cargo.toml'), '[package]\nname = "hey-db"\nversion = "0.1.0"\n');
  await assert.rejects(planVersion(directory, '0.1.2'), /Cargo app versions/);
  assert.equal(JSON.parse(await readFile(join(directory, 'package.json'))).version, '0.1.1');
});

test('replaces the stable filename and checksum together while preserving historical installers', async t => {
  const directory = await fixture(t);
  const downloads = join(directory, 'downloads');
  await mkdir(downloads);
  await writeFile(join(downloads, 'hey-db-macos-arm64.dmg'), 'previous installer');
  await writeFile(join(downloads, 'SHA256SUMS'), 'previous checksum');
  await writeFile(join(downloads, 'hey-db-0.1.0-macos-arm64.dmg'), 'historical installer');
  const source = join(directory, 'new.dmg');
  await writeFile(source, 'new installer bytes');
  await publishInstaller(source, downloads);
  const published = await readFile(join(downloads, 'hey-db-macos-arm64.dmg'));
  assert.equal(published.toString(), 'new installer bytes');
  assert.equal(await readFile(join(downloads, 'SHA256SUMS'), 'utf8'), `${createHash('sha256').update(published).digest('hex')}  hey-db-macos-arm64.dmg\n`);
  assert.equal(await readFile(join(downloads, 'hey-db-0.1.0-macos-arm64.dmg'), 'utf8'), 'historical installer');
  assert.deepEqual((await readdir(downloads)).sort(), ['SHA256SUMS', 'hey-db-0.1.0-macos-arm64.dmg', 'hey-db-macos-arm64.dmg']);
});

test('preserves the existing download and checksum if the build output is missing or empty', async t => {
  const directory = await fixture(t);
  const downloads = join(directory, 'downloads');
  await mkdir(downloads);
  await writeFile(join(downloads, 'hey-db-macos-arm64.dmg'), 'previous installer');
  await writeFile(join(downloads, 'SHA256SUMS'), 'previous checksum');
  await assert.rejects(publishInstaller(join(directory, 'missing.dmg'), downloads));
  await writeFile(join(directory, 'empty.dmg'), '');
  await assert.rejects(publishInstaller(join(directory, 'empty.dmg'), downloads), /nonempty/);
  assert.equal(await readFile(join(downloads, 'hey-db-macos-arm64.dmg'), 'utf8'), 'previous installer');
  assert.equal(await readFile(join(downloads, 'SHA256SUMS'), 'utf8'), 'previous checksum');
});
