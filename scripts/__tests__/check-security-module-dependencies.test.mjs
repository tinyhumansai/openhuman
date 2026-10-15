import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { checkManifest, checkRepository, HOST_MANIFESTS } from '../ci/check-security-module-dependencies.mjs';

const forms = [
  '[dependencies]\ntinysecurity-policy = "1"',
  '[build-dependencies]\nalias = { package = "tinysecurity-policy", version = "1" }',
  '[dev-dependencies.alias]\npackage = \'tinysecurity-crypto\'\nversion = "1"',
  '[target.\'cfg(windows)\'.dependencies]\nalias = {\n package = "tinysecurity-policy",\n version = "1"\n}',
  '[target.\'cfg(unix)\'.build-dependencies."alias"]\npackage = "tinysecurity-keyring"',
  '[target.\'cfg(unix)\'.dev-dependencies]\n"tinysecurity-policy" = "1"',
  '[workspace.dependencies]\nalias = { package = \'tinysecurity-policy\', version = "1" }',
  '[patch.crates-io]\nalias = { package = "tinysecurity-policy", path = "../policy" }',
  '[patch."https://github.com/example/repo".alias]\npackage = "tinysecurity-policy"\npath = "../policy"',
];
for (const [i, source] of forms.entries()) {
  test(`rejects forbidden security crate TOML form ${i}`, () => {
    assert.equal(checkManifest(source).length, 1);
  });
}
test('allows bus, alias to bus and unrelated packages', () => {
  assert.deepEqual(checkManifest('[dependencies]\ntinysecurity-bus = "1"\nfoo = { package = "tinysecurity-bus", version = "1" }\nother = "1"'), []);
});
test('package alias cannot hide forbidden crate behind bus name', () => {
  assert.equal(checkManifest('[dependencies]\ntinysecurity-bus = { package = "tinysecurity-policy", version = "1" }').length, 1);
});
test('rejects the internal umbrella crate and its aliases', () => {
  assert.equal(checkManifest('[dependencies]\ntinysecurity = "1"').length, 1);
  assert.equal(checkManifest('[dependencies]\ninternal = { package = "tinysecurity", version = "1" }').length, 1);
});
test('invalid TOML refuses a pass', () => assert.throws(() => checkManifest('[dependencies\n'), /./));

function fixture() {
  const root = mkdtempSync(join(tmpdir(), 'security-dependencies-'));
  for (const manifest of HOST_MANIFESTS) {
    mkdirSync(join(root, manifest, '..'), { recursive: true });
    writeFileSync(join(root, manifest), '[dependencies]\nserde = "1"\n');
  }
  return root;
}
test('repository examines root and host manifests while excluding vendor and worktrees', () => {
  const root = fixture();
  try {
    for (const excluded of ['vendor/module', 'worktrees/other']) {
      mkdirSync(join(root, excluded), { recursive: true });
      writeFileSync(join(root, excluded, 'Cargo.toml'), forms[0]);
    }
    assert.deepEqual(checkRepository(root), []);
    writeFileSync(join(root, 'crates/openhuman-cli/Cargo.toml'), forms[1]);
    assert.equal(checkRepository(root).length, 1);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
test('missing required manifest and unreadable root refuse vacuous success', () => {
  const root = fixture();
  try {
    rmSync(join(root, 'crates/openhuman-app/Cargo.toml'));
    assert.throws(() => checkRepository(root), /Cargo.toml/);
    assert.throws(() => checkRepository(join(root, 'absent')), /ENOENT/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
