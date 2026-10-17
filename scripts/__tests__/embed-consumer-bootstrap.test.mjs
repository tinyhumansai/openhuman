import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, writeFileSync, readFileSync, existsSync, rmSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const script = join(root, 'scripts/bootstrap-embed-consumer.py');
// CI hands the lanes their pinned interpreter explicitly; locally use PATH.
const python = process.env.OPENHUMAN_CI_PYTHON || 'python3';
// The toolcache interpreter is shared-linked and finds libpython only through
// LD_LIBRARY_PATH, which is no longer exported; give it to this child alone.
const env = process.env.OPENHUMAN_CI_PYTHON
  ? { ...process.env, LD_LIBRARY_PATH: [join(dirname(dirname(python)), 'lib'), process.env.LD_LIBRARY_PATH].filter(Boolean).join(':') }
  : process.env;
function fixture() {
  mkdirSync(join(root, 'target'), { recursive: true });
  const directory = mkdtempSync(join(root, 'target/embed-bootstrap-fixture-'));
  const source = join(directory, 'source');
  for (const path of ['crates/openhuman-embed/src', 'vendor/local-patch-fixture/src', 'vendor/unused-fixture/src'])
    mkdirSync(join(source, path), { recursive: true });
  writeFileSync(join(source, 'Cargo.toml'), '[workspace]\nmembers = ["crates/openhuman-embed", "vendor/local-patch-fixture"]\n[workspace.package]\nedition = "2024"\n[patch.crates-io]\nlocal-patch-fixture = { path = "vendor/local-patch-fixture" }\nunused-fixture = { path = "vendor/unused-fixture" }\n');
  writeFileSync(join(source, 'crates/openhuman-embed/Cargo.toml'), '[package]\nname = "openhuman-embed"\nversion = "0.0.0"\nedition.workspace = true\n[dependencies]\nlocal-patch-fixture = "1.0.0"\n');
  writeFileSync(join(source, 'crates/openhuman-embed/src/lib.rs'), '//! Fixture facade.\npub use local_patch_fixture::value;\n');
  writeFileSync(join(source, 'vendor/local-patch-fixture/Cargo.toml'), '[package]\nname = "local-patch-fixture"\nversion = "1.0.0"\nedition = "2024"\n');
  writeFileSync(join(source, 'vendor/local-patch-fixture/src/lib.rs'), '//! Fixture dependency.\npub fn value() -> u8 { 7 }\n');
  writeFileSync(join(source, 'vendor/unused-fixture/Cargo.toml'), '[package]\nname = "unused-fixture"\nversion = "1.0.0"\nedition = "2024"\n');
  writeFileSync(join(source, 'vendor/unused-fixture/src/lib.rs'), '//! Unused patch fixture.\n');
  const git = (...args) => execFileSync('git', ['-C', source, ...args], { encoding: 'utf8' }).trim();
  git('init', '--quiet');
  git('add', '.');
  git('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.org', 'commit', '--quiet', '-m', 'fixture');
  writeFileSync(join(source, '.env'), 'PRIVATE_FIXTURE_TOKEN=must-not-copy');
  return { directory, source, rev: git('rev-parse', 'HEAD') };
}

test('a pinned consumer generates patches, excludes local secrets and builds both feature modes offline', () => {
  const { directory, source, rev } = fixture();
  try {
    const destination = join(directory, 'consumer');
    execFileSync(python, [script, '--offline', '--source', source, '--rev', rev, '--destination', destination], { env });
    const manifest = readFileSync(join(destination, 'Cargo.toml'), 'utf8');
    assert.match(manifest, /default = \[\]/);
    assert.match(manifest, /optional = true, default-features = false/);
    assert.match(manifest, /vendor\/openhuman\/vendor\/local-patch-fixture/);
    assert.doesNotMatch(manifest, /unused-fixture/);
    assert.equal(readFileSync(join(destination, 'OPENHUMAN_REV'), 'utf8').trim(), rev);
    assert.equal(existsSync(join(destination, 'vendor/openhuman/.env')), false);
    assert.equal(existsSync(join(destination, 'vendor/openhuman/.git')), false);
    assert.equal(existsSync(join(source, '.git')), true);
    for (const features of [[], ['--features', 'embed']]) {
      const output = execFileSync(join(root, 'scripts/ci-cancel-aware.sh'), ['cargo', 'check', '--offline', '--locked', '--manifest-path', join(destination, 'Cargo.toml'), ...features], { cwd: root, encoding: 'utf8', stdio: 'pipe' });
      assert.equal(typeof output, 'string');
    }
    assert.doesNotMatch(readFileSync(join(destination, 'Cargo.lock'), 'utf8'), /patch\.unused/);
    const tree = execFileSync('cargo', ['tree', '--offline', '--manifest-path', join(destination, 'Cargo.toml'), '-e', 'normal'], { encoding: 'utf8' });
    assert.doesNotMatch(tree, /openhuman-embed|reqwest/);
    const again = spawnSync(python, [script, '--offline', '--source', source, '--rev', rev, '--destination', destination], { encoding: 'utf8', env });
    assert.equal(again.status, 1);
    assert.equal(readFileSync(join(destination, 'OPENHUMAN_REV'), 'utf8').trim(), rev);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});

test('symbolic and missing pins fail without leaving a partial consumer', () => {
  const { directory, source } = fixture();
  try {
    for (const rev of ['main', '0'.repeat(40)]) {
      const destination = join(directory, 'consumer');
      const result = spawnSync(python, [script, '--offline', '--source', source, '--rev', rev, '--destination', destination], { encoding: 'utf8', env });
      assert.equal(result.status, 1);
      assert.equal(existsSync(destination), false);
      assert.doesNotMatch(result.stderr, /PRIVATE_FIXTURE_TOKEN/);
    }
  } finally { rmSync(directory, { recursive: true, force: true }); }
});

test('credential-bearing source URLs are refused before Git sees them', () => {
  const { directory, source, rev } = fixture();
  try {
    const destination = join(directory, 'consumer');
    const result = spawnSync(python, [script, '--offline', '--source', `file://${source}?token=PRIVATE_FIXTURE_TOKEN`, '--rev', rev, '--destination', destination], { encoding: 'utf8', env });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /credential-bearing/);
    assert.doesNotMatch(result.stderr, /PRIVATE_FIXTURE_TOKEN/);
    assert.equal(existsSync(destination), false);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});

test('unsupported and escaping patch specifications fail closed', () => {
  for (const patch of ['{ git = "https://example.invalid/library" }', '{ path = "../../outside" }', '{ path = 3 }']) {
    const { directory, source } = fixture();
    try {
      writeFileSync(join(source, 'Cargo.toml'), `[workspace]\nmembers = ["crates/openhuman-embed", "vendor/local-patch-fixture"]\n[patch.crates-io]\nlocal-patch-fixture = ${patch}\n`);
      execFileSync('git', ['-C', source, 'add', 'Cargo.toml']);
      execFileSync('git', ['-C', source, '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.org', 'commit', '--quiet', '-m', 'invalid patch']);
      const rev = execFileSync('git', ['-C', source, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
      const destination = join(directory, 'consumer');
      const result = spawnSync(python, [script, '--offline', '--source', source, '--rev', rev, '--destination', destination], { encoding: 'utf8', env });
      assert.equal(result.status, 1);
      assert.match(result.stderr, /Unsupported workspace patch form|leaves the source checkout/);
      assert.equal(existsSync(destination), false);
    } finally { rmSync(directory, { recursive: true, force: true }); }
  }
});
