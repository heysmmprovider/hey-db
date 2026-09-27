import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFile, mkdir, mkdtemp, readFile, rename, rm, writeFile } from 'node:fs/promises';
import { arch, platform } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const installerName = 'hey-db-macos-arm64.dmg';
const target = 'aarch64-apple-darwin';
export const versionFiles = ['package.json', 'package-lock.json', 'src-tauri/tauri.conf.json', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock'];

function versionParts(value) {
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(value)) throw new Error('Use a stable version such as 0.1.2 (no v prefix).');
  const parts = value.split('.').map(Number);
  if (!parts.every(Number.isSafeInteger)) throw new Error('Version numbers are too large.');
  return parts;
}

export function nextVersion(current, requested) {
  const previous = versionParts(current);
  const version = requested ?? `${previous[0]}.${previous[1]}.${previous[2] + 1}`;
  const next = versionParts(version);
  const different = next.findIndex((part, index) => part !== previous[index]);
  if (different !== -1 && next[different] < previous[different]) throw new Error(`Version ${version} is older than ${current}.`);
  return version;
}

// Change only this app's Cargo entries, never a dependency with the same version.
function rustVersion(text, header, current, next) {
  const pattern = new RegExp(`(${header}\\r?\\nname = "hey-db"\\r?\\nversion = ")([^"]+)(")`);
  const found = text.match(pattern);
  if (!found || found[2] !== current) throw new Error('Cargo app versions must match package.json before releasing.');
  return text.replace(pattern, (_match, prefix, _version, suffix) => `${prefix}${next}${suffix}`);
}

export async function planVersion(directory, requested) {
  const originals = new Map(await Promise.all(versionFiles.map(async name => [name, await readFile(join(directory, name), 'utf8')])));
  const pkg = JSON.parse(originals.get('package.json'));
  const lock = JSON.parse(originals.get('package-lock.json'));
  const tauri = JSON.parse(originals.get('src-tauri/tauri.conf.json'));
  const current = pkg.version;
  const version = nextVersion(current, requested);
  if (lock.version !== current || lock.packages?.['']?.version !== current || tauri.version !== current) {
    throw new Error('The npm lockfile and Tauri version must match package.json before releasing.');
  }
  pkg.version = lock.version = lock.packages[''].version = tauri.version = version;
  const updates = new Map([
    ['package.json', `${JSON.stringify(pkg, null, 2)}\n`],
    ['package-lock.json', `${JSON.stringify(lock, null, 2)}\n`],
    ['src-tauri/tauri.conf.json', `${JSON.stringify(tauri, null, 2)}\n`],
    ['src-tauri/Cargo.toml', rustVersion(originals.get('src-tauri/Cargo.toml'), '\\[package\\]', current, version)],
    ['src-tauri/Cargo.lock', rustVersion(originals.get('src-tauri/Cargo.lock'), '\\[\\[package\\]\\]', current, version)],
  ]);
  return { current, version, originals, updates };
}

export async function publishInstaller(source, downloads) {
  const bytes = await readFile(source);
  if (!bytes.length || bytes.length >= 100 * 1024 * 1024) throw new Error('The installer must be nonempty and smaller than GitHub’s 100 MiB file limit.');
  const checksum = `${createHash('sha256').update(bytes).digest('hex')}  ${installerName}\n`;
  await mkdir(downloads, { recursive: true });
  const staging = await mkdtemp(join(downloads, '.release-'));
  const names = [installerName, 'SHA256SUMS'];
  const backups = new Map();
  let replacing = false;
  try {
    await writeFile(join(staging, installerName), bytes);
    await writeFile(join(staging, 'SHA256SUMS'), checksum);
    for (const name of names) {
      try { await copyFile(join(downloads, name), join(staging, `${name}.previous`)); backups.set(name, true); }
      catch (error) { if (error.code !== 'ENOENT') throw error; backups.set(name, false); }
    }
    replacing = true;
    for (const name of names) await rename(join(staging, name), join(downloads, name));
  } catch (error) {
    if (replacing) {
      for (const [name, existed] of backups) {
        if (existed) await copyFile(join(staging, `${name}.previous`), join(downloads, name));
        else await rm(join(downloads, name), { force: true });
      }
    }
    throw error;
  } finally { await rm(staging, { recursive: true, force: true }); }
  return checksum;
}

async function release(args) {
  if (args.length === 1 && ['--help', '-h'].includes(args[0])) {
    console.log('Usage: npm run release -- [0.1.2]\nWithout a version, increments the patch number. The current version can be rebuilt.\nBuilds locally, updates downloads/hey-db-macos-arm64.dmg and SHA256SUMS.\nDoes not stage, commit, tag, push, or publish anything.');
    return;
  }
  if (args.length > 1) throw new Error('Usage: npm run release -- [0.1.2]');
  if (platform() !== 'darwin' || arch() !== 'arm64') throw new Error('Run this release script using native Node.js on an Apple Silicon Mac.');
  const [nodeMajor, nodeMinor] = process.versions.node.split('.').map(Number);
  if (nodeMajor < 22 || (nodeMajor === 22 && nodeMinor < 12)) throw new Error('Node.js 22.12 or newer is required.');
  await mkdir(join(root, '.local'), { recursive: true });
  const lockfile = join(root, '.local', 'release.lock');
  try { await writeFile(lockfile, `${process.pid}\n`, { flag: 'wx' }); }
  catch (error) {
    if (error.code === 'EEXIST') throw new Error('A release is already running. If a previous run was killed, remove .local/release.lock after confirming it has stopped.');
    throw error;
  }
  let child;
  let interrupted = false;
  const stop = () => {
    interrupted = true;
    if (child?.pid) {
      try { process.kill(-child.pid, 'SIGTERM'); }
      catch (error) { if (error.code !== 'ESRCH') console.error(`Could not stop build: ${error.message}`); }
    }
  };
  process.on('SIGINT', stop);
  process.on('SIGTERM', stop);
  const run = async (command, commandArgs, extraEnv = {}, capture = false) => {
    if (interrupted) throw new Error('Release interrupted.');
    console.log(`\n> ${command} ${commandArgs.join(' ')}`);
    return new Promise((accept, reject) => {
      let output = '';
      child = spawn(command, commandArgs, {
        cwd: root, env: { ...process.env, ...extraEnv }, detached: true,
        stdio: capture ? ['inherit', 'pipe', 'inherit'] : 'inherit',
      });
      child.stdout?.on('data', data => { output += data; });
      child.once('error', error => { child = null; reject(error); });
      child.once('close', (code, signal) => {
        child = null;
        if (code === 0 && !interrupted) accept(output.trim());
        else reject(new Error(`${command} failed (${signal ?? code}).`));
      });
    });
  };
  let plan;
  let versionsWritten = false;
  let complete = false;
  try {
    plan = await planVersion(root, args[0]);
    await run('xcode-select', ['-p']);
    await run('cargo', ['--version']);
    await run('rustup', ['component', 'add', 'rustfmt', 'clippy']);
    console.log(`\nPreparing hey db ${plan.version}. The existing download stays in place until all checks pass.`);
    versionsWritten = true;
    for (const [name, contents] of plan.updates) await writeFile(join(root, name), contents);
    await run('npm', ['ci']);
    await run('npm', ['test']);
    await run('cargo', ['fmt', '--manifest-path', 'src-tauri/Cargo.toml', '--', '--check']);
    await run('cargo', ['clippy', '--locked', '--manifest-path', 'src-tauri/Cargo.toml', '--all-targets', '--', '-D', 'warnings']);
    await run('cargo', ['test', '--locked', '--manifest-path', 'src-tauri/Cargo.toml']);
    await run('npm', ['run', 'test:database']);
    const bundle = join(root, 'src-tauri', 'target', target, 'release', 'bundle');
    const dmg = join(bundle, 'dmg', `hey db_${plan.version}_aarch64.dmg`);
    const app = join(bundle, 'macos', 'hey db.app');
    // Remove the exact old build output so a stale installer cannot be published.
    await rm(dmg, { force: true });
    await run('npm', ['run', 'tauri', '--', 'build', '--target', target, '--bundles', 'app,dmg', '--ci', '--', '--locked'], {
      CI: 'true', APPLE_SIGNING_IDENTITY: process.env.APPLE_SIGNING_IDENTITY || '-',
      CARGO_TARGET_DIR: join(root, 'src-tauri', 'target'),
    });
    const builtVersion = await run('/usr/libexec/PlistBuddy', ['-c', 'Print:CFBundleShortVersionString', join(app, 'Contents', 'Info.plist')], {}, true);
    if (builtVersion !== plan.version) throw new Error(`Built app version is ${builtVersion}, expected ${plan.version}.`);
    const architecture = await run('lipo', ['-archs', join(app, 'Contents', 'MacOS', 'hey-db')], {}, true);
    if (architecture !== 'arm64') throw new Error(`Built app architecture is ${architecture}, expected arm64.`);
    await run('codesign', ['--verify', '--deep', '--strict', app]);
    await run('hdiutil', ['verify', dmg]);
    if (interrupted) throw new Error('Release interrupted.');
    await publishInstaller(dmg, join(root, 'downloads'));
    complete = true;
    console.log(`\nReady: hey db ${plan.version}\n  downloads/${installerName}\n  downloads/SHA256SUMS\n\nCommit the source/version changes, installer, and checksum together, then push to main yourself.\nYour existing website download URL stays unchanged. Nothing has been staged, committed, tagged, or pushed.`);
  } finally {
    try {
      if (versionsWritten && !complete) {
        for (const [name, contents] of plan.originals) await writeFile(join(root, name), contents);
        console.error('Release failed. Original versions restored; the previous download was preserved.');
      }
    } finally {
      process.off('SIGINT', stop); process.off('SIGTERM', stop);
      await rm(lockfile, { force: true });
    }
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  release(process.argv.slice(2)).catch(error => { console.error(`Release failed: ${error.message}`); process.exitCode = 1; });
}
