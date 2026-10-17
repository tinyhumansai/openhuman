import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import { ALLOWLIST, checkMetadata as checkMetadataAt, formatReport } from '../ci/check-storage-owner.mjs';

const CHECKER = resolve(dirname(fileURLToPath(import.meta.url)), '../ci/check-storage-owner.mjs');
const pkg = (name, deps = [], source = null, manifest_path) => ({
  name,
  source,
  manifest_path:
    manifest_path ??
    (name.startsWith('tinystoragedrivers')
      ? `/repo/vendor/tinystoragedrivers/crates/${name}/Cargo.toml`
      : `/repo/vendor/${name}/Cargo.toml`),
  dependencies: deps.map(d => ({ name: d, kind: null, optional: false })),
});
const checkMetadata = (m, root = '/repo') => checkMetadataAt(m, ALLOWLIST, root);
const registry = 'registry+https://github.com/rust-lang/crates.io-index';

test('a clean workspace passes', () => {
  assert.deepEqual(checkMetadata({ packages: [pkg('openhuman', ['serde']), pkg('tinyflows', ['tinystoragedrivers-core'])] }), []);
});

test('every forbidden backend dependency is rejected', () => {
  for (const dep of ['rusqlite', 'libsqlite3-sys', 'mongodb', 'object_store', 'sqlx', 'redb', 'sled', 'rocksdb']) {
    const v = checkMetadata({ packages: [pkg('tinymcp', [dep])] });
    assert.equal(v.length, 1, dep);
    assert.equal(v[0].package, 'tinymcp');
  }
});

test('tinystoragedrivers crates own the backends', () => {
  const v = checkMetadata({
    packages: [pkg('tinystoragedrivers-sqlite', ['rusqlite']), pkg('tinystoragedrivers-file')],
  });
  assert.deepEqual(v, []);
});

test('backend-named crates outside tinystoragedrivers are rejected', () => {
  for (const name of ['tinyflows-sqlite', 'tinyflows-mongodb', 'thing-file']) {
    assert.equal(checkMetadata({ packages: [pkg(name)] })[0].kind, 'crate-name', name);
  }
});

test('registry crates are ignored', () => {
  assert.deepEqual(
    checkMetadata({ packages: [pkg('rusqlite', ['libsqlite3-sys'], registry), pkg('same-file', [], registry)] }),
    []
  );
});

test('allowlisted read-only readers may depend on rusqlite, with a reason', () => {
  for (const [name, reason] of Object.entries(ALLOWLIST)) {
    assert.ok(reason.length > 10, `${name} needs a reason`);
    assert.deepEqual(checkMetadata({ packages: [pkg(name, ['rusqlite'])] }), []);
  }
  assert.equal(checkMetadata({ packages: [pkg('tinyskills', ['rusqlite'])] }).length, 1);
});

test('the report names the offender', () => {
  assert.match(formatReport(checkMetadata({ packages: [pkg('tinymcp', ['rusqlite'])] })), /tinymcp: depends on rusqlite/);
  assert.match(formatReport([]), /ok/);
});

test('CLI exits 1 on a violation, 0 when clean, 2 on unreadable input', () => {
  const dir = mkdtempSync(join(tmpdir(), 'storage-owner-'));
  try {
    const bad = join(dir, 'bad.json');
    const good = join(dir, 'good.json');
    writeFileSync(bad, JSON.stringify({ packages: [pkg('tinymcp', ['rusqlite'])] }));
    writeFileSync(good, JSON.stringify({ packages: [pkg('tinymcp')] }));
    assert.equal(spawnSync('node', [CHECKER, '--metadata', bad]).status, 1);
    assert.equal(spawnSync('node', [CHECKER, '--metadata', good]).status, 0);
    assert.equal(spawnSync('node', [CHECKER, '--metadata', join(dir, 'missing.json')]).status, 2);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('tinystoragedrivers packages under vendor/tinystoragedrivers pass', () => {
  assert.deepEqual(
    checkMetadata({ packages: [pkg('tinyflows', ['tinystoragedrivers-core']), pkg('tinystoragedrivers-core')] }),
    []
  );
});

test('a git or registry tinystoragedrivers package fails and names its dependents', () => {
  const git = 'git+https://github.com/tinyhumansai/tinystoragedrivers?tag=v0.12.0#abc';
  const v = checkMetadata({
    packages: [
      pkg('tinyflows', ['tinystoragedrivers-core']),
      pkg('tinystoragedrivers-core', [], git, '/home/u/.cargo/git/checkouts/tsd/abc/Cargo.toml'),
    ],
  });
  assert.equal(v.length, 1);
  assert.equal(v[0].kind, 'tsd-not-vendored');
  assert.match(formatReport(v), /tinystoragedrivers-core: resolves to git\+.*required by tinyflows.*bump vendor\/tinystoragedrivers/);
  const reg = checkMetadata({ packages: [pkg('tinystoragedrivers-core', [], registry)] });
  assert.equal(reg[0].kind, 'tsd-not-vendored');
});

test('a path tinystoragedrivers package outside the vendored checkout fails', () => {
  const v = checkMetadata({
    packages: [pkg('tinystoragedrivers-core', [], null, '/repo/vendor/tinyagents/vendor/tinystoragedrivers/crates/x/Cargo.toml')],
  });
  assert.equal(v.length, 1);
});

const tsd = (manifest_path, version = '0.11.0') => ({ ...pkg('tinystoragedrivers-core', [], null, manifest_path), version });
const TAG = 'git+https://github.com/tinyhumansai/tinystoragedrivers?tag=';
const dependsAt = (name, tag) => ({
  name,
  source: null,
  manifest_path: `/repo/vendor/${name}/Cargo.toml`,
  dependencies: [{ name: 'tinystoragedrivers-core', kind: null, optional: false, source: `${TAG}${tag}` }],
});

test('vendored path is anchored to this repository root', () => {
  const ok = '/repo/vendor/tinystoragedrivers/crates/core/Cargo.toml';
  assert.deepEqual(checkMetadata({ packages: [tsd(ok)] }), []);
  // a checkout below an ancestor named vendor is still accepted
  assert.deepEqual(checkMetadata({ packages: [tsd('/home/me/vendor/oh/vendor/tinystoragedrivers/c/Cargo.toml')] }, '/home/me/vendor/oh'), []);
  for (const bad of [
    '/other/vendor/tinystoragedrivers/crates/core/Cargo.toml',
    '/repo/vendor/x/vendor/tinystoragedrivers/crates/core/Cargo.toml',
    '/repo/other/vendor/tinystoragedrivers/crates/core/Cargo.toml',
  ]) {
    assert.equal(checkMetadata({ packages: [tsd(bad)] })[0]?.kind, 'tsd-not-vendored', bad);
  }
});

test('a module tag differing from the vendored version is rejected', () => {
  const vend = tsd('/repo/vendor/tinystoragedrivers/crates/core/Cargo.toml');
  assert.deepEqual(checkMetadata({ packages: [vend, dependsAt('tinyflows', 'v0.11.0')] }), []);
  const v = checkMetadata({ packages: [vend, dependsAt('tinyflows', 'v0.12.0')] });
  assert.equal(v.length, 1);
  assert.equal(v[0].kind, 'tsd-tag-mismatch');
  assert.match(v[0].detail, /v0\.12\.0.*0\.11\.0/);
});
