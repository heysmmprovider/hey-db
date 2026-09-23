import EmbeddedPostgres from 'embedded-postgres';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { randomBytes } from 'node:crypto';
import { createServer } from 'node:net';
import { spawn } from 'node:child_process';

const port = await new Promise((resolve, reject) => { const server = createServer(); server.on('error', reject); server.listen(0, '127.0.0.1', () => { const address = server.address(); server.close(() => resolve(address.port)); }); });
const dir = await mkdtemp(join(tmpdir(), 'heydb-test-'));
const password = randomBytes(24).toString('hex');
const pg = new EmbeddedPostgres({ databaseDir: join(dir, 'data'), user: 'heydb_test', password, port, persistent: false, authMethod: 'scram-sha-256', initdbFlags: ['--locale=C', '--encoding=UTF8'], postgresFlags: ['-h', '127.0.0.1'], onLog: () => {}, onError: () => {} });
let started = false;
try {
  await pg.initialise(); await pg.start(); started = true;
  await pg.createDatabase('heydb_test');
  console.log('Temporary PostgreSQL started. Running real database integration tests…');
  const status = await new Promise((resolve, reject) => { const child = spawn('cargo', ['test', '--manifest-path', 'src-tauri/Cargo.toml', '--test', 'database', '--', '--ignored', '--test-threads=1'], { stdio: 'inherit', env: { ...process.env, HEY_DB_TEST_PORT: String(port), HEY_DB_TEST_PASSWORD: password } }); child.once('error', reject); child.once('exit', code => resolve(code ?? 1)); });
  process.exitCode = status;
} finally { if (started) await pg.stop(); await rm(dir, { recursive: true, force: true }); }
