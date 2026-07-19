import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  diffForwarding,
  parseCoreDefaultFeatures,
  parseShellForwardedFeatures,
  stripComments,
} from '../lib/feature-forwarding.mjs';

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const CHECKER = resolve(REPO_ROOT, 'scripts/ci/check-feature-forwarding.mjs');

// ── parsing ────────────────────────────────────────────────────────────────

test('parses the core default gate list', () => {
  const toml = `
[features]
default = ["tokenjuice-treesitter", "voice", "media"]
voice = ["dep:hound"]
`;
  assert.deepEqual(parseCoreDefaultFeatures(toml), ['tokenjuice-treesitter', 'voice', 'media']);
});

test('parses a multi-line default gate list', () => {
  const toml = `
[features]
default = [
    "voice",
    "media",
]
`;
  assert.deepEqual(parseCoreDefaultFeatures(toml), ['voice', 'media']);
});

test('ignores a default key belonging to another table', () => {
  const toml = `
[some-other-table]
default = ["not-a-gate"]

[features]
default = ["voice"]
`;
  assert.deepEqual(parseCoreDefaultFeatures(toml), ['voice']);
});

test('parses the shell forwarded list across multiple lines', () => {
  const toml = `
openhuman_core = { path = "../..", package = "openhuman", default-features = false, features = [
    "media",
    "voice",
] }
`;
  assert.deepEqual(parseShellForwardedFeatures(toml), {
    defaultFeatures: false,
    features: ['media', 'voice'],
  });
});

test('detects when the shell inherits defaults instead of forwarding', () => {
  const toml = 'openhuman_core = { path = "../..", package = "openhuman" }\n';
  assert.deepEqual(parseShellForwardedFeatures(toml), { defaultFeatures: true, features: [] });
});

test('comment stripping does not truncate on a # inside a quoted value', () => {
  const stripped = stripComments('a = "issue #4901"  # trailing comment\n');
  assert.match(stripped, /issue #4901/);
  assert.doesNotMatch(stripped, /trailing comment/);
});

test('a commented-out gate does not count as forwarded', () => {
  const toml = `
openhuman_core = { path = "../..", package = "openhuman", default-features = false, features = [
    # "voice",
    "media",
] }
`;
  assert.deepEqual(parseShellForwardedFeatures(toml).features, ['media']);
});

// ── drift detection ────────────────────────────────────────────────────────

test('passes when every default gate is forwarded', () => {
  const result = diffForwarding({
    coreDefaults: ['voice', 'media'],
    shell: { defaultFeatures: false, features: ['media', 'voice'] },
  });
  assert.equal(result.ok, true);
  assert.deepEqual(result.missing, []);
});

test('reproduces #4901: a dropped voice gate is reported missing', () => {
  const result = diffForwarding({
    coreDefaults: ['tokenjuice-treesitter', 'voice', 'media'],
    shell: { defaultFeatures: false, features: ['media', 'tokenjuice-treesitter'] },
  });
  assert.equal(result.ok, false);
  assert.deepEqual(result.missing, ['voice']);
});

test('reproduces #4918: a dropped tokenjuice-treesitter gate is reported missing', () => {
  const result = diffForwarding({
    coreDefaults: ['tokenjuice-treesitter', 'voice', 'media'],
    shell: { defaultFeatures: false, features: ['media', 'voice'] },
  });
  assert.equal(result.ok, false);
  assert.deepEqual(result.missing, ['tokenjuice-treesitter']);
});

test('a brand new default gate is covered automatically, with no per-gate wiring', () => {
  const result = diffForwarding({
    coreDefaults: ['voice', 'media', 'some-future-gate'],
    shell: { defaultFeatures: false, features: ['voice', 'media'] },
  });
  assert.equal(result.ok, false);
  assert.deepEqual(result.missing, ['some-future-gate']);
});

test('an allow-listed gate passes and is reported as intentional', () => {
  const result = diffForwarding({
    coreDefaults: ['voice', 'heavy-gate'],
    shell: { defaultFeatures: false, features: ['voice'] },
    allowlist: { 'heavy-gate': 'Adds 400MB of models to the bundle.' },
  });
  assert.equal(result.ok, true);
  assert.deepEqual(result.allowed, ['heavy-gate']);
  assert.deepEqual(result.missing, []);
});

test('an allow-list entry for a gate that IS forwarded is flagged as stale', () => {
  const result = diffForwarding({
    coreDefaults: ['voice'],
    shell: { defaultFeatures: false, features: ['voice'] },
    allowlist: { voice: 'stale entry' },
  });
  assert.equal(result.ok, false);
  assert.deepEqual(result.stale, ['voice']);
});

test('inheriting defaults needs no forwarding', () => {
  const result = diffForwarding({
    coreDefaults: ['voice'],
    shell: { defaultFeatures: true, features: [] },
  });
  assert.equal(result.ok, true);
});

test('a missing dependency fails rather than passing vacuously', () => {
  const result = diffForwarding({ coreDefaults: ['voice'], shell: null });
  assert.equal(result.ok, false);
  assert.equal(result.reason, 'dependency-not-found');
});

// ── the real manifests + CLI ───────────────────────────────────────────────

test('the checked-in manifests pass the guard', () => {
  const out = execFileSync('node', [CHECKER], { encoding: 'utf8' });
  assert.match(out, /every default-ON core gate is forwarded/);
});

test('--help exits 0', () => {
  const out = execFileSync('node', [CHECKER, '--help'], { encoding: 'utf8' });
  assert.match(out, /Usage:/);
});

test('the real shell manifest forwards every real core default', () => {
  const coreDefaults = parseCoreDefaultFeatures(
    readFileSync(resolve(REPO_ROOT, 'Cargo.toml'), 'utf8')
  );
  const shell = parseShellForwardedFeatures(
    readFileSync(resolve(REPO_ROOT, 'app/src-tauri/Cargo.toml'), 'utf8')
  );
  // Guards the guard: if the parser silently returned nothing, the assertions
  // below would pass against empty input and prove nothing.
  assert.ok(coreDefaults.length > 0, 'expected to parse at least one core default gate');
  assert.equal(shell.defaultFeatures, false, 'shell is expected to set default-features = false');
  for (const gate of coreDefaults) {
    assert.ok(
      shell.features.includes(gate),
      `core default gate not forwarded to the shell: ${gate}`
    );
  }
});
