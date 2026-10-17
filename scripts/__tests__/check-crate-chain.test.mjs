import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  CHAIN,
  checkLinkedBridgeGate,
  checkManifestEdges,
  checkRepository,
  findForbiddenPaths,
  findPatternHits,
  CORE_WHOLESALE_REEXPORT_PATTERNS,
  HOST_RPC_INTERNAL_PATTERNS,
  RPC_INTERNAL_REEXPORT_PATTERNS,
  WHOLESALE_REEXPORT_PATTERNS,
  formatReport,
  parseNormalDependencies,
  parsePackageName,
  parseWorkspaceDependencies,
  parseWorkspaceMembers,
} from '../ci/check-crate-chain.mjs';

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const CHECKER = resolve(REPO_ROOT, 'scripts/ci/check-crate-chain.mjs');

// ── parsing ────────────────────────────────────────────────────────────────

test('reads the package name and workspace members', () => {
  assert.equal(parsePackageName('[package]\nname = "openhuman-tui" # the TUI\n'), 'openhuman-tui');
  assert.deepEqual(
    parseWorkspaceMembers(
      '[workspace]\nmembers = [\n  "crates/a",\n  # "crates/old",\n  "crates/b",\n]\n'
    ),
    ['crates/a', 'crates/b']
  );
});

test('normal dependencies include target tables and sub-tables, not dev/build ones', () => {
  const deps = parseNormalDependencies(`
[package]
name = "openhuman-cli"

[dependencies]
openhuman-rpc = { workspace = true, features = ["server"] }
core-alias = { path = "../openhuman-core", package = "openhuman", features = [
    "voice",
] }
serde = "1"

[target.'cfg(unix)'.dependencies]
openhuman-embed = { path = "../openhuman-embed" }

[dependencies.openhuman-tinyhumans]
path = "../openhuman-tinyhumans"

[dev-dependencies]
openhuman-core = { path = "../openhuman-core", package = "openhuman" }

[build-dependencies]
openhuman-tui = { path = "../openhuman-tui" }
`);
  assert.deepEqual(
    deps.map(d => d.package),
    ['openhuman-rpc', 'openhuman', 'serde', 'openhuman-embed', 'openhuman-tinyhumans']
  );
  assert.equal(deps[0].workspace, true);
});

test('a commented-out dependency is not an edge', () => {
  const deps = parseNormalDependencies(
    '[dependencies]\n# openhuman-core = { path = "x" }\nlog = "0.4"\n'
  );
  assert.deepEqual(
    deps.map(d => d.key),
    ['log']
  );
});

test('workspace dependencies resolve their package rename', () => {
  const ws = parseWorkspaceDependencies(`
[workspace.dependencies]
openhuman-core = { path = "crates/openhuman-core", package = "openhuman" }
openhuman-rpc = { path = "crates/openhuman-rpc", default-features = false }
`);
  assert.equal(ws.get('openhuman-core'), 'openhuman');
  assert.equal(ws.get('openhuman-rpc'), 'openhuman-rpc');
});

// ── the chain ──────────────────────────────────────────────────────────────

test('each layer may name only the layer directly below it', () => {
  assert.deepEqual(CHAIN['openhuman-embed'], ['openhuman']);
  assert.deepEqual(CHAIN['openhuman-cli'], ['openhuman-rpc', 'openhuman-modules-linked']);
  assert.deepEqual(
    checkManifestEdges({
      crate: 'openhuman-tui',
      deps: [{ key: 'openhuman-rpc', package: 'openhuman-rpc', workspace: false }],
    }),
    []
  );
});

test('a host depending on the core is a violation, even through a workspace alias', () => {
  const workspaceDeps = new Map([['openhuman-core', 'openhuman']]);
  const violations = checkManifestEdges({
    crate: 'openhuman-tui',
    deps: [
      { key: 'openhuman-rpc', package: 'openhuman-rpc', workspace: true },
      { key: 'openhuman-core', package: 'openhuman-core', workspace: true },
      { key: 'openhuman-tinyhumans', package: 'openhuman-tinyhumans', workspace: false },
    ],
    workspaceDeps,
  });
  assert.deepEqual(
    violations.map(v => v.dependency),
    ['openhuman', 'openhuman-tinyhumans']
  );
});

test('the core may not depend on any OpenHuman crate', () => {
  const [violation] = checkManifestEdges({
    crate: 'openhuman',
    deps: [{ key: 'openhuman-rpc', package: 'openhuman-rpc', workspace: false }],
  });
  assert.match(violation.reason, /may not depend on any OpenHuman crate/);
});

test('an unknown OpenHuman crate fails until it is placed in the chain', () => {
  const [violation] = checkManifestEdges({ crate: 'openhuman-new', deps: [] });
  assert.match(violation.reason, /unknown OpenHuman crate/);
});

test('host source naming core internals by path is flagged, with file and line', () => {
  const hits = findForbiddenPaths(
    'use openhuman_rpc::embed::config;\nlet x = openhuman_core::config::Config::default();\nuse crate::core_host::agent;\nuse openhuman_embed::__host::core;\n',
    'src/lib.rs'
  );
  assert.deepEqual(
    hits.map(h => `${h.line}:${h.pattern}`),
    ['2:openhuman_core::', '3:core_host', '4:__host']
  );
});

test('a test name that merely contains openhuman_core is not a path', () => {
  assert.deepEqual(
    findForbiddenPaths('fn binary_path_result_contains_openhuman_core() {}', 'x.rs'),
    []
  );
});

test('a layer re-exporting the layer below wholesale is flagged', () => {
  const flagged = text =>
    findPatternHits(text, 'lib.rs', WHOLESALE_REEXPORT_PATTERNS).map(h => h.pattern);
  assert.deepEqual(flagged('pub use openhuman_embed as embed;\n'), [
    'pub use openhuman_embed as …',
  ]);
  assert.deepEqual(flagged('pub use openhuman_tinyhumans as tinyhumans;\n'), [
    'pub use openhuman_tinyhumans as …',
  ]);
  assert.deepEqual(flagged('pub use openhuman_tinyhumans::embed;\n'), [
    'pub use openhuman_tinyhumans::embed (the crate, not a list)',
  ]);
  assert.deepEqual(flagged('pub use openhuman_embed::*;\n'), ['pub use openhuman_embed::*']);
  assert.deepEqual(flagged('pub use openhuman_embed;\n'), [
    'pub use openhuman_embed / openhuman_tinyhumans (the bare crate)',
  ]);
  // Curated lists, private aliases and comments are fine.
  assert.deepEqual(flagged('pub use openhuman_embed::{Runtime, RuntimeBuilder};\n'), []);
  assert.deepEqual(flagged('use openhuman_embed as embed;\n'), []);
  assert.deepEqual(flagged('// pub use openhuman_embed as embed;\n'), []);
  assert.deepEqual(flagged('/* pub use openhuman_embed::*; */\n'), []);
});

test('embed may not re-export the core wholesale, and grouped wholesale forms are flagged', () => {
  const core = text =>
    findPatternHits(text, 'lib.rs', CORE_WHOLESALE_REEXPORT_PATTERNS).map(h => h.line);
  assert.deepEqual(core('pub use openhuman_core as core;\n'), [1]);
  assert.deepEqual(core('pub use openhuman_core::*;\n'), [1]);
  assert.deepEqual(core('pub use openhuman_core::{self, agent};\n'), [1]);
  assert.deepEqual(core('pub use openhuman_core::agent::turn_origin::AgentTurnOrigin;\n'), []);
  assert.deepEqual(core('pub use openhuman_core::{CoreBuilder, CoreRuntime};\n'), []);
  const grouped = text =>
    findPatternHits(text, 'lib.rs', WHOLESALE_REEXPORT_PATTERNS).map(h => h.line);
  assert.deepEqual(grouped('pub use openhuman_embed::{self as embed};\n'), [1]);
  assert.deepEqual(grouped('pub use openhuman_tinyhumans::{SessionManager, CoreLink};\n'), []);
});

test('rpc may not re-export the internal list on a public path', () => {
  const flagged = text =>
    findPatternHits(text, 'lib.rs', RPC_INTERNAL_REEXPORT_PATTERNS).map(h => `${h.line}`);
  assert.deepEqual(flagged('pub use openhuman_tinyhumans::__host as core_host;\n'), ['1']);
  assert.deepEqual(
    flagged(
      'pub mod embed {\n    pub use openhuman_tinyhumans::embed::{\n        config,\n        __host,\n    };\n}\n'
    ),
    ['2']
  );
  assert.deepEqual(flagged('pub(crate) use openhuman_tinyhumans::__host as core_host;\n'), []);
  // A public re-export of a module beneath the list is still the list leaking.
  assert.deepEqual(flagged('pub use crate::core_host::config;\n'), ['1']);
  // `unwrap_rpc` is the one item rpc passes up from the list.
  assert.deepEqual(flagged('pub use crate::core_host::core::unwrap_rpc;\n'), []);
});

test('a host, root test or example naming the internal list is flagged, aliased or not', () => {
  const flagged = text =>
    findPatternHits(text, 'main.rs', HOST_RPC_INTERNAL_PATTERNS).map(h => `${h.line}`);
  assert.deepEqual(flagged('use openhuman_rpc::embed::__host::config;\n'), ['1']);
  assert.deepEqual(flagged('use openhuman_rpc as rpc;\nuse rpc::core_host::agent;\n'), ['2']);
  assert.deepEqual(flagged('use openhuman_rpc::embed::config;\n'), []);
});

test('char literals do not desynchronize the stripper; grouped embed is flagged', () => {
  const flagged = text =>
    findPatternHits(text, 'lib.rs', WHOLESALE_REEXPORT_PATTERNS).map(h => h.line);
  assert.deepEqual(flagged("let t = s.trim_matches('\"');\npub use openhuman_embed as e;\n"), [2]);
  assert.deepEqual(flagged('pub use openhuman_tinyhumans::{embed, RuntimeBuilder};\n'), [1]);
  assert.deepEqual(
    flagged('pub use openhuman_tinyhumans::{embed::process, RuntimeBuilder};\n'),
    []
  );
  const internal = text =>
    findPatternHits(text, 'lib.rs', RPC_INTERNAL_REEXPORT_PATTERNS).map(h => h.line);
  assert.deepEqual(internal('pub use crate::core_host::{core::unwrap_rpc, secret};\n'), [1]);
});

test('string literals and comments are not code', () => {
  const flagged = text =>
    findPatternHits(text, 'lib.rs', WHOLESALE_REEXPORT_PATTERNS).map(h => h.pattern);
  assert.deepEqual(flagged('log::warn!("pub use openhuman_embed as embed;");\n'), []);
  assert.deepEqual(flagged('let s = r#"pub use openhuman_embed::*;"#;\n'), []);
  assert.deepEqual(flagged('let s = "a \\" pub use openhuman_embed as e;";\n'), []);
  assert.deepEqual(flagged('/* a /* nested */ pub use openhuman_embed as e; */\n'), []);
  assert.deepEqual(flagged('"x";\npub use openhuman_embed as embed;\n'), [
    'pub use openhuman_embed as …',
  ]);
});

// ── the real repository ────────────────────────────────────────────────────

test('the checked-in manifests and host sources hold the chain', () => {
  const result = checkRepository(REPO_ROOT);
  assert.ok(result.checked.length >= 7, `checked only ${result.checked.join(', ')}`);
  assert.ok(
    result.checked.includes('openhuman-app'),
    'the desktop shell is outside the workspace and must still be checked'
  );
  assert.deepEqual(result.edgeViolations, [], formatReport(result));
  assert.deepEqual(result.sourceViolations, [], formatReport(result));
  assert.deepEqual(result.facadeViolations, [], formatReport(result));
});

test('the CLI exits 1 and names the edge when a host reaches past rpc', () => {
  const dir = mkdtempSync(join(tmpdir(), 'crate-chain-'));
  try {
    writeFileSync(
      join(dir, 'Cargo.toml'),
      '[workspace]\nmembers = ["crates/openhuman-core", "crates/openhuman-tui"]\n'
    );
    const crate = (name, toml) => {
      mkdirSync(join(dir, 'crates', name, 'src'), { recursive: true });
      writeFileSync(join(dir, 'crates', name, 'Cargo.toml'), toml);
    };
    crate('openhuman-core', '[package]\nname = "openhuman"\n');
    crate(
      'openhuman-tui',
      '[package]\nname = "openhuman-tui"\n[dependencies]\nopenhuman-core = { path = "../openhuman-core", package = "openhuman" }\n'
    );
    crate(
      'openhuman-app',
      '[package]\nname = "openhuman-app"\n[dependencies]\nopenhuman-rpc = { path = "../openhuman-rpc" }\n'
    );
    crate('openhuman-cli', '[package]\nname = "openhuman-cli"\n');
    writeFileSync(
      join(dir, 'crates/openhuman-cli/src/main.rs'),
      'fn main() { openhuman_core::run(); }\n'
    );
    for (const layer of ['openhuman-embed', 'openhuman-tinyhumans', 'openhuman-rpc']) {
      mkdirSync(join(dir, 'crates', layer, 'src'), { recursive: true });
    }
    writeFileSync(
      join(dir, 'crates/openhuman-rpc/src/lib.rs'),
      'pub use openhuman_tinyhumans as tinyhumans;\n'
    );
    const result = spawnSync('node', [CHECKER, dir], { encoding: 'utf8' });
    assert.equal(result.status, 1, result.stdout + result.stderr);
    assert.match(result.stderr, /openhuman-tui -> openhuman: may only depend on openhuman-rpc/);
    assert.match(result.stderr, /crates\/openhuman-cli\/src\/main\.rs:1: openhuman_core::/);
    assert.match(
      result.stderr,
      /crates\/openhuman-rpc\/src\/lib\.rs:1: pub use openhuman_tinyhumans as/
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('the CLI exits 2 instead of passing when it cannot read the repository', () => {
  const dir = mkdtempSync(join(tmpdir(), 'crate-chain-empty-'));
  try {
    const result = spawnSync('node', [CHECKER, dir], { encoding: 'utf8' });
    assert.equal(result.status, 2, result.stdout + result.stderr);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('the real repository passes through the CLI', () => {
  const result = spawnSync('node', [CHECKER], { encoding: 'utf8' });
  assert.equal(result.status, 0, result.stdout + result.stderr);
  assert.match(result.stdout, /OK: every crate depends only on the layer below it/);
});

// ── the linked-modules bridge (E0.2) ───────────────────────────────────────

const BRIDGE_CLI = `
[package]
name = "openhuman-cli"
[dependencies]
openhuman-rpc = { workspace = true }
openhuman-modules-linked = { path = "../openhuman-modules-linked", features = ["all"], optional = true }
[features]
default = ["modules"]
modules = ["openhuman-rpc/modules"]
linked-modules = ["modules", "dep:openhuman-modules-linked"]
`;

test('only the CLI may name the linked-modules bridge; every other crate breaks the chain', () => {
  const bridge = [{ key: 'openhuman-modules-linked', package: 'openhuman-modules-linked', workspace: false }];
  assert.deepEqual(checkManifestEdges({ crate: 'openhuman-cli', deps: bridge }), []);
  for (const crate of ['openhuman-app', 'openhuman-tui', 'openhuman', 'openhuman-embed', 'openhuman-rpc']) {
    const violations = checkManifestEdges({ crate, deps: bridge });
    assert.equal(violations.length, 1, crate);
    assert.equal(violations[0].dependency, 'openhuman-modules-linked');
  }
});

test('the bridge depends on rpc alone', () => {
  const deps = [{ key: 'openhuman-embed', package: 'openhuman-embed', workspace: false }];
  assert.equal(checkManifestEdges({ crate: 'openhuman-modules-linked', deps }).length, 1);
});

test('an optional bridge behind linked-modules passes the CLI gate', () => {
  assert.deepEqual(checkLinkedBridgeGate(BRIDGE_CLI), []);
  assert.deepEqual(checkLinkedBridgeGate('[package]\nname = "openhuman-cli"\n'), []);
});

test('a non-optional bridge dependency in the CLI fails the gate', () => {
  const toml = BRIDGE_CLI.replace(', optional = true', '');
  const violations = checkLinkedBridgeGate(toml);
  assert.ok(violations.some(v => /optional = true/.test(v.reason)), JSON.stringify(violations));
});

test('enabling the bridge from default or another feature fails the gate', () => {
  const viaDefault = BRIDGE_CLI.replace('default = ["modules"]', 'default = ["modules", "linked-modules"]');
  assert.ok(checkLinkedBridgeGate(viaDefault).some(v => /feature `default`/.test(v.reason)));
  const viaOther = BRIDGE_CLI.replace('modules = ["openhuman-rpc/modules"]', 'modules = ["openhuman-rpc/modules", "dep:openhuman-modules-linked"]');
  assert.ok(checkLinkedBridgeGate(viaOther).some(v => /feature `modules`/.test(v.reason)));
});

test('a bridge with no linked-modules feature fails the gate', () => {
  const toml = BRIDGE_CLI.replace(/linked-modules = .*\n/, '');
  assert.ok(checkLinkedBridgeGate(toml).some(v => /must enable it/.test(v.reason)));
});

test('the CLI exits 1 when the TUI depends on the linked-modules bridge', () => {
  const dir = mkdtempSync(join(tmpdir(), 'crate-chain-bridge-'));
  try {
    writeFileSync(
      join(dir, 'Cargo.toml'),
      '[workspace]\nmembers = ["crates/openhuman-tui", "crates/openhuman-cli"]\n'
    );
    const crate = (name, toml) => {
      mkdirSync(join(dir, 'crates', name, 'src'), { recursive: true });
      writeFileSync(join(dir, 'crates', name, 'Cargo.toml'), toml);
    };
    crate(
      'openhuman-tui',
      '[package]\nname = "openhuman-tui"\n[dependencies]\nopenhuman-modules-linked = { path = "../openhuman-modules-linked" }\n'
    );
    crate('openhuman-cli', BRIDGE_CLI);
    crate('openhuman-app', '[package]\nname = "openhuman-app"\n');
    for (const layer of ['openhuman-embed', 'openhuman-tinyhumans', 'openhuman-rpc']) {
      mkdirSync(join(dir, 'crates', layer, 'src'), { recursive: true });
    }
    const result = spawnSync('node', [CHECKER, dir], { encoding: 'utf8' });
    assert.equal(result.status, 1, result.stdout + result.stderr);
    assert.match(result.stderr, /openhuman-tui -> openhuman-modules-linked/);
    assert.doesNotMatch(result.stderr, /openhuman-cli -> openhuman-modules-linked/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
