import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  CHAIN_GATES_NOT_FORWARDED,
  CHAIN_LOCAL_GATES,
  checkProductForwarding,
  diffChainForwarding,
  diffForwarding,
  formatChainReport,
  INTENTIONALLY_NOT_FORWARDED,
  parseCoreDefaultFeatures,
  parseCoreFeatureGraph,
  parseCoreFeatureNames,
  parseFeatureTable,
  parseProductFeatures,
  parseShellForwardedFeatures,
  resolveEnabledFeatures,
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

// ── product-set forwarding (assertions 1 + 2) ──────────────────────────────

const PRODUCT = ['voice', 'media'];
const CORE_GATES = ['voice', 'media', 'web3', 'tui'];

test('passes when the shell forwards exactly the product set', () => {
  const result = checkProductForwarding({
    productFeatures: PRODUCT,
    coreFeatureNames: CORE_GATES,
    shell: { defaultFeatures: false, features: ['media', 'voice'] },
  });
  assert.equal(result.ok, true);
});

test('reproduces #4901 against the PRODUCT set, not the default set', () => {
  // The point of the rewrite: this must fail even though `default` here is
  // empty, which is precisely the shape that made the old subset check pass
  // vacuously as `default` shrank.
  const result = checkProductForwarding({
    productFeatures: PRODUCT,
    coreFeatureNames: CORE_GATES,
    shell: { defaultFeatures: false, features: ['media'] },
  });
  assert.equal(result.ok, false);
  assert.deepEqual(result.missing, ['voice']);
});

test('a gate the shell forwards but the product does not claim is flagged', () => {
  const result = checkProductForwarding({
    productFeatures: PRODUCT,
    coreFeatureNames: CORE_GATES,
    shell: { defaultFeatures: false, features: ['media', 'voice', 'web3'] },
  });
  assert.equal(result.ok, false);
  assert.deepEqual(result.unexpected, ['web3']);
});

test('a product gate that is not a real core gate is flagged', () => {
  const result = checkProductForwarding({
    productFeatures: ['voice', 'vioce'],
    coreFeatureNames: CORE_GATES,
    shell: { defaultFeatures: false, features: ['voice', 'vioce'] },
  });
  assert.equal(result.ok, false);
  assert.deepEqual(result.unknown, ['vioce']);
});

test('the shell inheriting defaults is now a FAILURE, not a pass', () => {
  // It used to mean "nothing to drift". It now means the shell would inherit
  // the contributor set, which is smaller than the product.
  const result = checkProductForwarding({
    productFeatures: PRODUCT,
    coreFeatureNames: CORE_GATES,
    shell: { defaultFeatures: true, features: [] },
  });
  assert.equal(result.ok, false);
  assert.equal(result.reason, 'shell-inherits-defaults');
});

test('parses the product file, ignoring comments and blank lines', () => {
  const text = '# a comment\n\nvoice\n  media  # trailing\n\n';
  assert.deepEqual(parseProductFeatures(text), ['voice', 'media']);
});

test('parses every gate name from the core [features] table, minus `default`', () => {
  const toml = `
[features]
default = ["media"]
voice = ["dep:hound"]
media = []

[dependencies]
hound = "3"
`;
  assert.deepEqual(parseCoreFeatureNames(toml), ['voice', 'media']);
});

// ── the real manifests + CLI ───────────────────────────────────────────────

test('the checked-in manifests pass the guard', () => {
  const out = execFileSync('node', [CHECKER], { encoding: 'utf8' });
  assert.match(out, /the shell forwards exactly the product gate set/);
  assert.match(out, /every default-ON core gate is forwarded/);
});

test('the real product file and the real shell list are equal', () => {
  const productFeatures = parseProductFeatures(
    readFileSync(resolve(REPO_ROOT, 'scripts/ci/product-features.txt'), 'utf8')
  );
  const coreFeatureNames = parseCoreFeatureNames(
    readFileSync(resolve(REPO_ROOT, 'crates/openhuman-core/Cargo.toml'), 'utf8')
  );
  const shell = parseShellForwardedFeatures(
    readFileSync(resolve(REPO_ROOT, 'crates/openhuman-app/Cargo.toml'), 'utf8')
  );
  // Guards the guard: empty input would make every assertion below vacuous.
  assert.ok(productFeatures.length > 0, 'expected to parse at least one product gate');
  assert.ok(coreFeatureNames.length > 0, 'expected to parse at least one core gate name');
  const result = checkProductForwarding({ productFeatures, coreFeatureNames, shell });
  assert.deepEqual(result.missing, [], 'product gates the shell does not forward');
  assert.deepEqual(result.unexpected, [], 'gates the shell forwards that the product omits');
  assert.deepEqual(result.unknown, [], 'product gates that are not real core gates');
});

test('the shell helper reports an empty gate list instead of dying silently', () => {
  // Regression. The helper filters comments with `grep -v`, which exits 1 when
  // it selects nothing; under `set -e` that aborted the script INSIDE the
  // command substitution, so a comments-only file exited 1 with no output at
  // all and the explicit diagnostic below it was unreachable. A CI lane would
  // have seen a bare failure with nothing naming the cause.
  const tmp = join(tmpdir(), `product-features-empty-${process.pid}.txt`);
  writeFileSync(tmp, '# only a comment\n\n   \n');
  try {
    const result = spawnSync(
      'bash',
      [resolve(REPO_ROOT, 'scripts/ci/product-features.sh'), tmp],
      { encoding: 'utf8' }
    );
    assert.equal(result.status, 2, 'an empty gate list must exit 2, not 1');
    assert.match(result.stderr, /empty gate list/);
    assert.equal(result.stdout.trim(), '', 'nothing may be emitted for an empty list');
  } finally {
    rmSync(tmp, { force: true });
  }
});

test('the shell helper parses a fixture the same way the JS parser does', () => {
  const tmp = join(tmpdir(), `product-features-fixture-${process.pid}.txt`);
  writeFileSync(tmp, '# heading\n\nvoice\n  media  # trailing comment\n\nweb3\n');
  try {
    const out = execFileSync(
      'bash',
      [resolve(REPO_ROOT, 'scripts/ci/product-features.sh'), tmp],
      { encoding: 'utf8' }
    ).trim();
    assert.equal(out, 'voice,media,web3');
    assert.deepEqual(out.split(','), parseProductFeatures(readFileSync(tmp, 'utf8')));
  } finally {
    rmSync(tmp, { force: true });
  }
});

test('the shell script and the JS parser agree on the product set', () => {
  // Two parsers read scripts/ci/product-features.txt: this one, and the shell
  // helper the CI lanes use to build `--features`. If they disagreed, CI would
  // compile a different set than the guard asserts — and the guard would be
  // checking something nobody builds.
  const fromJs = parseProductFeatures(
    readFileSync(resolve(REPO_ROOT, 'scripts/ci/product-features.txt'), 'utf8')
  );
  const fromSh = execFileSync('bash', [resolve(REPO_ROOT, 'scripts/ci/product-features.sh')], {
    encoding: 'utf8',
  })
    .trim()
    .split(',');
  assert.deepEqual(fromSh, fromJs);
});

test('--help exits 0', () => {
  const out = execFileSync('node', [CHECKER, '--help'], { encoding: 'utf8' });
  assert.match(out, /Usage:/);
});

test('the real shell manifest forwards every real core default', () => {
  const coreDefaults = parseCoreDefaultFeatures(
    readFileSync(resolve(REPO_ROOT, 'crates/openhuman-core/Cargo.toml'), 'utf8')
  );
  const shell = parseShellForwardedFeatures(
    readFileSync(resolve(REPO_ROOT, 'crates/openhuman-app/Cargo.toml'), 'utf8')
  );
  // Guards the guard: if the parser silently returned nothing, the assertions
  // below would pass against empty input and prove nothing.
  assert.ok(coreDefaults.length > 0, 'expected to parse at least one core default gate');
  assert.equal(shell.defaultFeatures, false, 'shell is expected to set default-features = false');
  for (const gate of coreDefaults) {
    // Gates the shell intentionally does not forward (e.g. `tui` — a terminal
    // subcommand the desktop app never runs) are exempt, matching the checker.
    if (INTENTIONALLY_NOT_FORWARDED[gate]) continue;
    assert.ok(
      shell.features.includes(gate),
      `core default gate not forwarded to the shell: ${gate}`
    );
  }
});

// ── feature graph ──────────────────────────────────────────────────────────

test('parses the whole feature table, default included', () => {
  const toml = `
[features]
default = ["media", "modules"]
documents = ["modules", "dep:tinydocs-bus"]
modules = ["tinybus/modules"]

[dependencies]
serde = "1"
documents = ["not-a-feature"]
`;
  const graph = parseCoreFeatureGraph(toml);

  assert.deepEqual(graph.get('default'), ['media', 'modules']);
  assert.deepEqual(graph.get('documents'), ['modules', 'dep:tinydocs-bus']);
  // Bounded at the next table header, so a `[dependencies]` key of the same
  // name cannot overwrite a real gate's dependency list.
  assert.equal(graph.size, 3);
});

test('resolves a gate enabled only through another gate', () => {
  const graph = parseCoreFeatureGraph(`
[features]
default = ["documents"]
documents = ["modules"]
modules = []
`);

  const enabled = resolveEnabledFeatures(graph, ['default']);

  // `modules` is nowhere in `default`; a direct membership test would read it
  // as OFF while cargo compiles it in.
  assert.ok(enabled.has('modules'), 'expected a transitively enabled gate to resolve as ON');
  assert.ok(enabled.has('documents'));
  assert.ok(!enabled.has('voice'), 'expected an unrelated gate to stay OFF');
});

test('does not mistake dependency activations for local gates', () => {
  const graph = parseCoreFeatureGraph(`
[features]
default = ["modules"]
modules = ["tinybus/modules", "dep:ureq"]
`);

  const enabled = resolveEnabledFeatures(graph, ['default']);

  // `tinybus/modules` forwards a feature into a dependency and `dep:ureq` turns
  // an optional dependency on. Neither names a gate in THIS crate, so neither
  // can be what a local `#[cfg(feature = "…")]` reads.
  assert.ok(!enabled.has('tinybus/modules'));
  assert.ok(!enabled.has('dep:ureq'));
  assert.ok(enabled.has('modules'));
});

test('seeds beyond default are followed too', () => {
  const graph = parseCoreFeatureGraph(`
[features]
default = []
documents = ["modules"]
modules = []
`);

  // The product set is a second seed alongside `default`: the e2e runner passes
  // `--features` WITHOUT `--no-default-features`, so the measured build is the
  // union of the two.
  const enabled = resolveEnabledFeatures(graph, ['default', 'documents']);

  assert.ok(enabled.has('documents'));
  assert.ok(enabled.has('modules'));
});

test('a seed the feature table does not declare resolves to itself', () => {
  const enabled = resolveEnabledFeatures(parseCoreFeatureGraph('[features]\ndefault = []\n'), [
    'default',
    'ghost',
  ]);

  // A typo in product-features.txt is caught by `checkProductForwarding`, not
  // here; this must not throw on the way there.
  assert.ok(enabled.has('ghost'));
});

test('a manifest with no [features] table yields an empty graph', () => {
  assert.equal(parseCoreFeatureGraph('[package]\nname = "openhuman"\n').size, 0);
});

test('a cycle in the feature graph terminates instead of hanging', () => {
  // Cargo would reject this, but the gate reads the file as text and must not
  // spin on a hand-edit that has not been through cargo yet.
  const graph = parseCoreFeatureGraph(`
[features]
default = ["a"]
a = ["b"]
b = ["a"]
`);

  const enabled = resolveEnabledFeatures(graph, ['default']);

  assert.deepEqual([...enabled].sort(), ['a', 'b', 'default']);
});

test('reads TOML literal strings, not just basic strings', () => {
  // Both forms are valid TOML and cargo accepts either. Matching only `"…"`
  // reported these arrays as EMPTY, and empty is the answer that makes every
  // consumer here pass vacuously. (CodeRabbit, PR #6092.)
  assert.deepEqual(parseCoreDefaultFeatures("[features]\ndefault = ['voice', \"media\"]\n"), [
    'voice',
    'media',
  ]);
  assert.deepEqual(parseCoreFeatureGraph("[features]\ndefault = ['documents']\ndocuments = ['modules']\n").get('documents'), ['modules']);
  assert.deepEqual(
    parseShellForwardedFeatures(
      "openhuman_core = { path = \"../..\", default-features = false, features = ['voice'] }\n",
    ).features,
    ['voice'],
  );
});

test('an apostrophe inside a basic string does not open a literal string', () => {
  // Alternation order is load-bearing: `"…"` is tried first at each position,
  // so the `'` in `don't` is consumed as part of the basic string rather than
  // starting a literal one and swallowing the rest of the array.
  assert.deepEqual(parseCoreDefaultFeatures('[features]\ndefault = ["don\'t", "media"]\n'), [
    "don't",
    'media',
  ]);
});

// ── the library chain: core → embed → tinyhumans → cli (#6364) ─────────────

const CORE_CHAIN_FIXTURE = `
[features]
default = ["media"]
media = []
voice = ["dep:cpal"]
e2e-test-support = []
`;

function chainLink(toml, sources, extra = {}) {
  return diffChainForwarding({
    crate: 'openhuman-embed',
    features: parseFeatureTable(toml),
    sources,
    ...extra,
  });
}

function coreSource() {
  return {
    crate: 'openhuman-core',
    gates: [...parseFeatureTable(CORE_CHAIN_FIXTURE).keys()].filter(n => n !== 'default'),
    required: true,
  };
}

test('a layer that forwards every core gate passes', () => {
  const result = chainLink(
    `
[features]
default = ["openhuman-core/default"]
media = ["openhuman-core/media"]
voice = ["openhuman-core/voice"]
e2e-test-support = ["openhuman-core/e2e-test-support"]
`,
    [coreSource()]
  );
  assert.ok(result.ok, formatChainReport(result));
});

test('a core gate the layer forgets is reported missing, naming crate and gate', () => {
  // The silent direction: `--features voice` still resolves on openhuman-cli,
  // so CI stays green while the gate is off for every embedder.
  const result = chainLink(
    `
[features]
default = ["openhuman-core/default"]
media = ["openhuman-core/media"]
e2e-test-support = ["openhuman-core/e2e-test-support"]
`,
    [coreSource()]
  );
  assert.equal(result.ok, false);
  assert.deepEqual(result.missing, [{ gate: 'voice', source: 'openhuman-core' }]);
  const report = formatChainReport(result);
  assert.match(report, /openhuman-embed/);
  assert.match(report, /voice \(declared by openhuman-core\)/);
});

test('reproduces #6360: a gate the core dropped but a layer still forwards is flagged', () => {
  const result = chainLink(
    `
[features]
default = ["openhuman-core/default"]
media = ["openhuman-core/media"]
voice = ["openhuman-core/voice"]
e2e-test-support = ["openhuman-core/e2e-test-support"]
peripheral-rpi = ["openhuman-core/peripheral-rpi"]
`,
    [coreSource()]
  );
  assert.equal(result.ok, false);
  assert.deepEqual(result.unknown, ['peripheral-rpi']);
  assert.match(formatChainReport(result), /no crate below it has/);
});

test('a forward that names the wrong gate is caught, not just a missing one', () => {
  // A name-only check reads this as forwarded. It is not: enabling `voice`
  // would turn `web3` on in the core and leave voice compiled out.
  const result = chainLink(
    `
[features]
default = ["openhuman-core/default"]
media = ["openhuman-core/media"]
voice = ["openhuman-core/web3"]
e2e-test-support = ["openhuman-core/e2e-test-support"]
`,
    [coreSource()]
  );
  assert.equal(result.ok, false);
  assert.deepEqual(result.misrouted, [
    { gate: 'voice', source: 'openhuman-core', expected: 'openhuman-core/voice' },
  ]);
  assert.match(formatChainReport(result), /expected "openhuman-core\/voice"/);
});

test('an allow-listed omission passes and is reported as intentional', () => {
  const notForwarded = { 'e2e-test-support': 'Destructive test_reset RPC; never for embedders.' };
  const result = chainLink(
    `
[features]
default = ["openhuman-core/default"]
media = ["openhuman-core/media"]
voice = ["openhuman-core/voice"]
`,
    [coreSource()],
    { notForwarded }
  );
  assert.ok(result.ok, formatChainReport(result, { notForwarded }));
  assert.deepEqual(result.allowed, ['e2e-test-support']);
  assert.match(formatChainReport(result, { notForwarded }), /allowed: e2e-test-support/);
});

test('an allow-list entry for a gate that IS forwarded is flagged as stale', () => {
  const result = chainLink(
    `
[features]
default = ["openhuman-core/default"]
media = ["openhuman-core/media"]
voice = ["openhuman-core/voice"]
e2e-test-support = ["openhuman-core/e2e-test-support"]
`,
    [coreSource()],
    { notForwarded: { 'e2e-test-support': 'never for embedders' } }
  );
  assert.equal(result.ok, false);
  assert.deepEqual(result.staleExclusion, ['e2e-test-support']);
});

test('an exclusion for a gate no crate below declares any more is flagged as stale', () => {
  // The other half of the stale check: once the core drops the gate, nothing
  // declares it, so it stops appearing in the `allowed:` report too — and the
  // entry would silently exclude a gate by that name if one ever came back.
  const result = chainLink(
    `
[features]
default = ["openhuman-core/default"]
media = ["openhuman-core/media"]
voice = ["openhuman-core/voice"]
e2e-test-support = ["openhuman-core/e2e-test-support"]
`,
    [coreSource()],
    { notForwarded: { 'peripheral-rpi': 'a gate the core deleted' } }
  );
  assert.equal(result.ok, false);
  assert.deepEqual(result.staleExclusion, ['peripheral-rpi']);
  assert.match(formatChainReport(result), /no crate below declares it any more/);
});

test('a crate-local entry for a gate this crate no longer declares is flagged as stale', () => {
  const result = chainLink(
    `
[features]
default = ["openhuman-core/default"]
media = ["openhuman-core/media"]
voice = ["openhuman-core/voice"]
e2e-test-support = ["openhuman-core/e2e-test-support"]
`,
    [coreSource()],
    { localGates: { jev: 'This crate owns the gate.' } }
  );
  assert.equal(result.ok, false);
  assert.deepEqual(result.staleLocal, ['jev']);
  assert.match(formatChainReport(result), /no longer does/);
});

test('a documented crate-local gate passes', () => {
  const result = chainLink(
    `
[features]
default = ["openhuman-core/default"]
media = ["openhuman-core/media"]
voice = ["openhuman-core/voice"]
e2e-test-support = ["openhuman-core/e2e-test-support"]
jev = ["dep:tinytools-jev"]
`,
    [coreSource()],
    { localGates: { jev: 'This crate owns the gate.' } }
  );
  assert.ok(result.ok, formatChainReport(result));
  assert.deepEqual(result.unknown, []);
});

test('a crate-local entry for a gate the core now declares is flagged as stale', () => {
  // Otherwise the entry would excuse a missing forward for a real core gate.
  const result = chainLink(
    `
[features]
default = ["openhuman-core/default"]
media = ["openhuman-core/media"]
voice = ["openhuman-core/voice"]
e2e-test-support = ["openhuman-core/e2e-test-support"]
`,
    [coreSource()],
    { localGates: { voice: 'local, honest' } }
  );
  assert.equal(result.ok, false);
  assert.deepEqual(result.staleLocal, ['voice']);
});

test('a forward to a gate the crate below no longer has is named as drift', () => {
  // #6360 verbatim: cargo rejects this, but only for whoever builds that crate
  // first, and the error reads as a dependency problem.
  const result = diffChainForwarding({
    crate: 'openhuman-cli',
    features: parseFeatureTable(`
[features]
default = ["openhuman-core/default"]
media = ["openhuman-core/media", "openhuman-tinyhumans/media"]
voice = ["openhuman-core/voice", "openhuman-tinyhumans/voice"]
e2e-test-support = ["openhuman-core/e2e-test-support"]
`),
    sources: [
      coreSource(),
      // tinyhumans dropped `voice`; the cli still points at it.
      { crate: 'openhuman-tinyhumans', gates: ['media'], required: false },
    ],
  });
  assert.equal(result.ok, false);
  assert.deepEqual(result.dangling, [{ gate: 'voice', item: 'openhuman-tinyhumans/voice' }]);
  assert.match(formatChainReport(result), /no longer exists/);
});

test('the cli must forward to tinyhumans as well where tinyhumans has the gate', () => {
  const result = diffChainForwarding({
    crate: 'openhuman-cli',
    features: parseFeatureTable(`
[features]
default = ["openhuman-core/default"]
media = ["openhuman-core/media"]
voice = ["openhuman-core/voice"]
e2e-test-support = ["openhuman-core/e2e-test-support"]
`),
    sources: [
      coreSource(),
      { crate: 'openhuman-tinyhumans', gates: ['media', 'voice'], required: false },
    ],
  });
  assert.equal(result.ok, false);
  assert.deepEqual(
    result.misrouted.map(m => m.expected),
    ['openhuman-tinyhumans/media', 'openhuman-tinyhumans/voice']
  );
});

test('an optional source never demands a gate the crate below it does not have', () => {
  // `e2e-test-support` is core-only: requiring `openhuman-tinyhumans/e2e-test-support`
  // would be a forward to a gate that does not exist, i.e. a cargo error.
  const result = diffChainForwarding({
    crate: 'openhuman-cli',
    features: parseFeatureTable(`
[features]
default = ["openhuman-core/default"]
media = ["openhuman-core/media", "openhuman-tinyhumans/media"]
voice = ["openhuman-core/voice", "openhuman-tinyhumans/voice"]
e2e-test-support = ["openhuman-core/e2e-test-support"]
`),
    sources: [
      coreSource(),
      { crate: 'openhuman-tinyhumans', gates: ['media', 'voice'], required: false },
    ],
  });
  assert.ok(result.ok, formatChainReport(result));
});

test('the checked-in embed, tinyhumans and cli manifests forward the whole chain', () => {
  const read = name => readFileSync(resolve(REPO_ROOT, `crates/${name}/Cargo.toml`), 'utf8');
  const core = parseCoreFeatureNames(
    readFileSync(resolve(REPO_ROOT, 'crates/openhuman-core/Cargo.toml'), 'utf8')
  );
  const embed = parseFeatureTable(read('openhuman-embed'));
  const tinyhumans = parseFeatureTable(read('openhuman-tinyhumans'));
  const cli = parseFeatureTable(read('openhuman-cli'));
  // Guards the guard: empty tables would make every assertion below vacuous.
  assert.ok(core.length > 0, 'expected to parse at least one core gate');
  for (const [name, table] of [
    ['openhuman-embed', embed],
    ['openhuman-tinyhumans', tinyhumans],
    ['openhuman-cli', cli],
  ]) {
    assert.ok(table.size > 0, `expected to parse features from ${name}`);
  }
  const gatesOf = table => [...table.keys()].filter(n => n !== 'default');
  const links = [
    {
      crate: 'openhuman-embed',
      features: embed,
      sources: [{ crate: 'openhuman-core', gates: core, required: true }],
    },
    {
      crate: 'openhuman-tinyhumans',
      features: tinyhumans,
      sources: [{ crate: 'openhuman-embed', gates: gatesOf(embed), required: true }],
    },
    {
      crate: 'openhuman-cli',
      features: cli,
      sources: [
        { crate: 'openhuman-core', gates: core, required: true },
        { crate: 'openhuman-tinyhumans', gates: gatesOf(tinyhumans), required: false },
      ],
    },
  ];
  for (const link of links) {
    const notForwarded = CHAIN_GATES_NOT_FORWARDED[link.crate] ?? {};
    const result = diffChainForwarding({
      ...link,
      notForwarded,
      localGates: CHAIN_LOCAL_GATES[link.crate] ?? {},
    });
    assert.ok(result.ok, formatChainReport(result, { notForwarded }));
  }
});

test('every chain allow-list entry carries a reason', () => {
  // A blank reason is how "excluded on purpose" and "forgotten" start looking
  // the same again — the ambiguity that let #4918 sit unnoticed.
  for (const table of [CHAIN_GATES_NOT_FORWARDED, CHAIN_LOCAL_GATES]) {
    for (const [crate, gates] of Object.entries(table)) {
      for (const [gate, reason] of Object.entries(gates)) {
        assert.equal(typeof reason, 'string', `${crate}/${gate}`);
        assert.ok(reason.trim().length > 10, `${crate}/${gate} needs a real reason`);
      }
    }
  }
});

test('the checker reports the chain and fails when a layer drops a gate', () => {
  // End-to-end through the CLI: a core gate that embed never took must exit 1
  // with the crate and gate named.
  const dir = join(tmpdir(), `feature-chain-${process.pid}`);
  const corePath = join(dir, 'core.toml');
  const embedPath = join(dir, 'embed.toml');
  try {
    mkdirSync(dir, { recursive: true });
    writeFileSync(
      corePath,
      readFileSync(resolve(REPO_ROOT, 'crates/openhuman-core/Cargo.toml'), 'utf8')
    );
    writeFileSync(
      embedPath,
      readFileSync(resolve(REPO_ROOT, 'crates/openhuman-embed/Cargo.toml'), 'utf8').replace(
        /^voice = .*$/m,
        ''
      )
    );
    const result = spawnSync(
      'node',
      [
        CHECKER,
        corePath,
        resolve(REPO_ROOT, 'crates/openhuman-app/Cargo.toml'),
        resolve(REPO_ROOT, 'scripts/ci/product-features.txt'),
        embedPath,
        resolve(REPO_ROOT, 'crates/openhuman-tinyhumans/Cargo.toml'),
        resolve(REPO_ROOT, 'crates/openhuman-cli/Cargo.toml'),
      ],
      { encoding: 'utf8' }
    );
    assert.equal(result.status, 1, result.stdout + result.stderr);
    assert.match(result.stdout, /Library chain/);
    assert.match(result.stdout, /openhuman-embed/);
    assert.match(result.stdout, /voice \(declared by openhuman-core\)/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
