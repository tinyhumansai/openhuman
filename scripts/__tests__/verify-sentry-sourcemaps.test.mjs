import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const source = join(dirname(fileURLToPath(import.meta.url)), '../release/verify-sentry-sourcemaps.mjs');

function run(bundle) {
  const root = mkdtempSync(join(tmpdir(), 'sentry-debug-id-test-'));
  try {
    const script = join(root, 'scripts/release/verify-sentry-sourcemaps.mjs');
    const assets = join(root, 'app/dist/assets');
    mkdirSync(dirname(script), { recursive: true });
    mkdirSync(assets, { recursive: true });
    copyFileSync(source, script);
    if (bundle !== null) writeFileSync(join(assets, 'app.js'), bundle);
    return spawnSync(process.execPath, [script], { encoding: 'utf8' });
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

test('a plain SDK property reference is not debug-ID injection', () => {
  const result = run('const debugIdMap = GLOBAL_OBJ._sentryDebugIds;');
  assert.equal(result.status, 1);
  assert.match(result.stderr, /no Sentry debug-ID injection found/);
  assert.equal(run('//# debugId=------------------------------------').status, 1);
});

test('accepts the injection snippet emitted by the locked Sentry plugin', () => {
  // getDebugIdSnippet() in @sentry/bundler-plugin-core 2.23.1 (pnpm lockfile).
  const result = run(';!function(){try{var e="undefined"!=typeof window?window:"undefined"!=typeof global?global:"undefined"!=typeof globalThis?globalThis:"undefined"!=typeof self?self:{},n=(new e.Error).stack;n&&(e._sentryDebugIds=e._sentryDebugIds||{},e._sentryDebugIds[n]="12345678-1234-4234-8234-123456789abc",e._sentryDebugIdIdentifier="sentry-dbid-12345678-1234-4234-8234-123456789abc")}catch(e){}}();');
  assert.equal(result.status, 0);
  assert.match(result.stdout, /upload and symbolication were not verified/);
});

test('accepts a debug-ID pragma and distinguishes missing bundles', () => {
  assert.equal(run('//# debugId=12345678-1234-4234-8234-123456789abc').status, 0);
  assert.equal(run(null).status, 2);
});
