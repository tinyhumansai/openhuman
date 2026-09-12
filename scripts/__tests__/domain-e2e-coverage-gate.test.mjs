// Behavioural cover for `scripts/check-domain-e2e-coverage.mjs` (#5936).
//
// The sibling `coverage-script-help.test.mjs` only asserts `--help` and
// argument rejection, so all three defects #5936 fixed were unpinned: the gate
// could go back to measuring a seventh of the surface and every test would stay
// green. That is the failure mode this file exists to make loud.
//
// Each test drives the real script against a fixture tree — the script keys off
// `process.cwd()`, so a temp dir with `src/openhuman/**` and `tests/**` is a
// complete world — and asserts on the specific defect, not on the exit code
// alone. Exit status is deliberately NOT the assertion where it cannot
// discriminate: an incomplete fixture trips the `declaredButMissing` guard, so
// several of these runs exit non-zero for more than one reason.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';

const HERE = dirname(fileURLToPath(import.meta.url));
const SCRIPT = resolve(HERE, '..', 'check-domain-e2e-coverage.mjs');

function write(root, relative, contents) {
  const full = join(root, relative);
  fs.mkdirSync(dirname(full), { recursive: true });
  fs.writeFileSync(full, contents);
}

/** A minimal `ControllerSchema` literal in the shape the scanner matches. */
function controller(namespace, fn) {
  return `
pub const SCHEMA: ControllerSchema = ControllerSchema {
    namespace: "${namespace}",
    function: "${fn}",
};
`;
}

function runGate(root, threshold = '90') {
  return spawnSync(process.execPath, [SCRIPT], {
    cwd: root,
    encoding: 'utf8',
    env: { ...process.env, DOMAIN_E2E_COVERAGE_THRESHOLD: threshold },
  });
}

/** A `[features]` table in the shape `parseCoreFeatureGraph` reads. */
function cargoToml(defaultFeatures, extraFeatures) {
  const quote = (items) => items.map((item) => `"${item}"`).join(', ');
  const lines = ['[features]', `default = [${quote(defaultFeatures)}]`];
  for (const [name, deps] of Object.entries(extraFeatures)) lines.push(`${name} = [${quote(deps)}]`);
  return `${lines.join('\n')}\n`;
}

/**
 * A fixture world.
 *
 * Two things every fixture needs that a bare temp dir does not have. The gate
 * hard-requires `Cargo.toml` and `scripts/ci/product-features.txt`, because
 * that pair is what `scripts/test-rust-e2e.sh` builds its `--features` string
 * from and therefore the only honest answer to "which configuration is being
 * measured". And — unless a test is proving the stale-entry guard — it needs a
 * declaration for every `UNREACHABLE_NAMESPACES` entry, so a run is not
 * tripping over an exclusion that names nothing in this world.
 */
function fixture(t, options = {}) {
  const {
    defaultFeatures = [],
    productFeatures = [],
    featureGraph = {},
    withExcludedNamespaces = true,
    declareExcludedFeatures = true,
    excludedModuleCfg = '#[cfg(feature = "e2e-test-support")]',
  } = options;
  const root = fs.mkdtempSync(join(tmpdir(), 'domain-e2e-gate-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  // The manifest must DECLARE each exclusion's gate even when nothing enables
  // it — an undeclared gate is a rename the table missed, not a disabled one,
  // and the gate refuses it.
  const features = declareExcludedFeatures ? { 'e2e-test-support': [], ...featureGraph } : featureGraph;
  write(root, 'Cargo.toml', cargoToml(defaultFeatures, features));
  write(root, 'scripts/ci/product-features.txt', `${productFeatures.join('\n')}\n`);
  if (withExcludedNamespaces) {
    write(
      root,
      'src/openhuman/test_support/schemas.rs',
      controller('test', 'reset') + controller('test_support', 'workspace_root'),
    );
    // A faithful module tree: `schemas.rs` is reached through `mod schemas;`,
    // and `test_support` through the declaration in its parent. The `#[cfg]`
    // the exclusion claims lives on the SECOND of those — which is the whole
    // reason the gate walks the chain instead of reading the file it found the
    // controller in.
    write(root, 'src/openhuman/test_support/mod.rs', 'mod schemas;\n');
    write(
      root,
      'src/openhuman/mod.rs',
      `${excludedModuleCfg ? `${excludedModuleCfg}\n` : ''}pub mod test_support;\n`,
    );
  }
  return root;
}

// Defect (a): discovery used to read only files whose path matched
// `/(^|\/)schemas?(\.rs|\/)/`. The #5856/#5857 `include!` split moved
// `ControllerSchema` literals into `*_part_NN.rs` siblings, which that pattern
// does not match — 180 controllers across 12 namespaces went invisible in one
// commit and the gate simply reported a smaller world.
test('discovers controllers declared in an include!-split part file', (t) => {
  const root = fixture(t);
  // The filename is the point: `schemas_part_01.rs` does NOT match the old
  // path filter, because `schemas` is followed by `_` rather than `.rs` or `/`.
  write(root, 'src/openhuman/widgets/schemas_part_01.rs', controller('widgets', 'list'));
  write(root, 'tests/widgets_e2e.rs', 'let m = "openhuman.widgets_list";');

  const result = runGate(root);

  assert.match(
    result.stdout,
    /Discovered 1 controllers across 1 namespaces/,
    `the part file's controller must be discovered; got:\n${result.stdout}`,
  );
  assert.match(
    result.stdout,
    /\| widgets \| widgets \| 1\/1 \| 100\.0% \|/,
    `widgets must be measured at 1/1; got:\n${result.stdout}`,
  );
});

// Defect (b): `percent = expected.size === 0 ? 100 : …` meant a namespace whose
// controllers had all become invisible scored 100% — indistinguishable in the
// gate's own output from genuinely full coverage. Four namespaces were doing
// exactly that. A namespace MODULES names but discovery cannot see is now a
// hard failure, because nothing was measured.
test('fails loudly when a declared namespace discovers no controllers', (t) => {
  const root = fixture(t);
  write(root, 'src/openhuman/widgets/schemas_part_01.rs', controller('widgets', 'list'));
  write(root, 'tests/widgets_e2e.rs', 'let m = "openhuman.widgets_list";');

  const result = runGate(root);

  assert.equal(result.status, 1, `an unmeasurable namespace must fail the gate; got:\n${result.stdout}`);
  assert.match(
    result.stderr,
    /namespace\(s\) with no discovered controllers/,
    `the failure must say nothing was measured; got:\n${result.stderr}`,
  );
  // `config` is a MODULES entry with no controller in this fixture. Scoring it
  // 100% is the bug; naming it as unmeasured is the fix.
  assert.match(result.stderr, /\bconfig\b/, `the unmeasured namespace must be named; got:\n${result.stderr}`);
  assert.doesNotMatch(
    result.stdout,
    /\| config \|.*100\.0% \|/,
    `an unmeasured namespace must never be reported as 100% covered; got:\n${result.stdout}`,
  );
});

// Defect (c): MODULES used to be the *scope* of the check, so ~50 namespaces
// were never measured at any threshold simply because nobody added a line.
// MODULES is now presentational; every discovered namespace is measured.
test('measures a discovered namespace that MODULES does not name', (t) => {
  const root = fixture(t);
  // `widgets` appears nowhere in MODULES.
  write(root, 'src/openhuman/widgets/schemas_part_01.rs', controller('widgets', 'list'));
  write(root, 'src/openhuman/widgets/schemas_part_02.rs', controller('widgets', 'purge'));
  // Only one of the two is named by an e2e target: 1/2 = 50%, under the 90% bar.
  write(root, 'tests/widgets_e2e.rs', 'let m = "openhuman.widgets_list";');

  const result = runGate(root);

  assert.match(
    result.stdout,
    /\| widgets \| widgets \| 1\/2 \| 50\.0% \|/,
    `an unlisted namespace must still be measured; got:\n${result.stdout}`,
  );
  assert.match(
    result.stderr,
    /widgets \(1\/2, 50\.0%\)/,
    `an unlisted namespace below the threshold must fail the gate by name; got:\n${result.stderr}`,
  );
  assert.match(
    result.stdout,
    /openhuman\.widgets_purge/,
    `the uncovered controller must be reported as missing; got:\n${result.stdout}`,
  );
});

// #6069: the gate demanded coverage of methods that are not compiled into the
// build it measures. `test` / `test_support` sit behind `e2e-test-support`,
// which is in neither `[features] default` nor product-features.txt, so the
// only ways to satisfy those rows were a bespoke feature string or naming the
// method in a string literal that never calls it — the exact gaming the
// `collectInvokedMethods` header warns about. The honest answer (0%,
// unreachable) and the dishonest one (0%, nobody bothered) looked identical.
test('excludes namespaces compiled out of the measured configuration', (t) => {
  const root = fixture(t);
  write(root, 'src/openhuman/widgets/schemas_part_01.rs', controller('widgets', 'list'));
  write(root, 'tests/widgets_e2e.rs', 'let m = "openhuman.widgets_list";');

  const result = runGate(root);

  assert.doesNotMatch(
    result.stdout,
    /^\| test \|/m,
    `an unreachable namespace must not be billed as an uncovered obligation; got:\n${result.stdout}`,
  );
  assert.doesNotMatch(
    result.stdout,
    /^\| test_support \|/m,
    `an unreachable namespace must not be billed as an uncovered obligation; got:\n${result.stdout}`,
  );
  // The denominator must shrink with the rows. A row hidden from the table but
  // still counted would be a worse report than the bug.
  assert.match(
    result.stdout,
    /Discovered 1 controllers across 1 namespaces/,
    `excluded controllers must leave the denominator; got:\n${result.stdout}`,
  );
});

// Excluded is not the same as forgotten. The report has to show the claim —
// "nothing here can be dispatched" — or nobody ever reviews it.
test('reports what it excluded, with the gate and the reason', (t) => {
  const root = fixture(t);
  write(root, 'src/openhuman/widgets/schemas_part_01.rs', controller('widgets', 'list'));
  write(root, 'tests/widgets_e2e.rs', 'let m = "openhuman.widgets_list";');

  const result = runGate(root);

  assert.match(
    result.stdout,
    /Excluded 2 controller\(s\) in 2 namespace\(s\) as unreachable/,
    `the exclusion must be stated, not silent; got:\n${result.stdout}`,
  );
  assert.match(
    result.stdout,
    /test \(1\) — compiled out by `e2e-test-support`:/,
    `each exclusion must name its gate; got:\n${result.stdout}`,
  );
  assert.match(
    result.stdout,
    /test_support \(1\) — compiled out by `e2e-test-support`:/,
    `each exclusion must name its gate; got:\n${result.stdout}`,
  );
});

// The guard that makes the list safe to keep. An exclusion is a claim about the
// measured build, and if that build starts compiling the family in, the claim
// deletes real obligations from the denominator and reports the smaller world
// as success — strictly worse than the bug #6069 fixed.
test('fails when an excluded namespace becomes reachable in the product set', (t) => {
  const root = fixture(t, { productFeatures: ['e2e-test-support'] });
  write(root, 'src/openhuman/widgets/schemas_part_01.rs', controller('widgets', 'list'));
  write(root, 'tests/widgets_e2e.rs', 'let m = "openhuman.widgets_list";');

  const result = runGate(root);

  assert.equal(result.status, 1, `a stale exclusion must fail the gate; got:\n${result.stdout}`);
  assert.match(
    result.stderr,
    /excluded namespace\(s\) are reachable in the measured configuration/,
    `the failure must say the exclusion no longer holds; got:\n${result.stderr}`,
  );
  assert.match(
    result.stderr,
    /test \(gated on "e2e-test-support"\)/,
    `the offending namespace and gate must be named; got:\n${result.stderr}`,
  );
});

// Cargo features are transitive, so "is the gate in `default` or the product
// list" is the wrong question — `documents = ["modules", …]` turns `modules` on
// for anyone enabling `documents`. A direct-membership check passes this
// fixture while cargo compiles the family in, which is why the gate resolves
// the graph instead.
test('fails when an excluded namespace is reachable only transitively', (t) => {
  const root = fixture(t, {
    defaultFeatures: ['bundle'],
    featureGraph: { bundle: ['e2e-test-support'], 'e2e-test-support': [] },
  });
  write(root, 'src/openhuman/widgets/schemas_part_01.rs', controller('widgets', 'list'));
  write(root, 'tests/widgets_e2e.rs', 'let m = "openhuman.widgets_list";');

  const result = runGate(root);

  assert.equal(result.status, 1, `a transitively-enabled gate must fail the gate; got:\n${result.stdout}`);
  assert.match(
    result.stderr,
    /excluded namespace\(s\) are reachable in the measured configuration/,
    `following the feature graph is the point of this test; got:\n${result.stderr}`,
  );
});

// The other way the list rots: the namespace is renamed or deleted and the
// entry silently stops referring to anything. Same failure `declaredButMissing`
// catches for MODULES, applied to the exclusion list.
test('fails when an excluded namespace no longer exists', (t) => {
  const root = fixture(t, { withExcludedNamespaces: false });
  write(root, 'src/openhuman/widgets/schemas_part_01.rs', controller('widgets', 'list'));
  write(root, 'tests/widgets_e2e.rs', 'let m = "openhuman.widgets_list";');

  const result = runGate(root);

  assert.equal(result.status, 1, `an exclusion naming nothing must fail the gate; got:\n${result.stdout}`);
  assert.match(
    result.stderr,
    /UNREACHABLE_NAMESPACES names 2 namespace\(s\) with no discovered controllers: test, test_support/,
    `the stale entries must be named; got:\n${result.stderr}`,
  );
});

// Without both files the gate cannot say which configuration it is measuring,
// and an exclusion nothing verifies is the silent hole the list exists to
// close. Refusing is the only honest answer — exit 2, the same usage-error
// status as a bad threshold.
test('refuses to run without the files the measured feature set comes from', (t) => {
  const root = fixture(t);
  write(root, 'src/openhuman/widgets/schemas_part_01.rs', controller('widgets', 'list'));
  fs.rmSync(join(root, 'scripts', 'ci', 'product-features.txt'));

  const result = runGate(root);

  assert.equal(result.status, 2, `a world with no product feature list must be refused; got:\n${result.stdout}`);
  assert.match(
    result.stderr,
    /product-features\.txt not found/,
    `the failure must name the missing file; got:\n${result.stderr}`,
  );
});

// Guarding the guard. A `[features]` table the parser cannot see does not fail
// on its own — it reports every gate as OFF, which is exactly the answer that
// makes every exclusion look earned. Silence there would undo the check.
test('refuses to run when the feature table cannot be parsed', (t) => {
  const root = fixture(t);
  write(root, 'src/openhuman/widgets/schemas_part_01.rs', controller('widgets', 'list'));
  write(root, 'Cargo.toml', '[package]\nname = "openhuman"\n');

  const result = runGate(root);

  assert.equal(result.status, 2, `an unparseable feature table must be refused; got:\n${result.stdout}`);
  assert.match(
    result.stderr,
    /no `\[features\] default` in Cargo\.toml/,
    `the failure must name what could not be resolved; got:\n${result.stderr}`,
  );
});

// Codex, PR #6092. The subtlest way this list rots: a gate is renamed, every
// `#[cfg]` site is updated, and only this table is missed. The old name is then
// absent from the feature graph — so it is absent from the enabled set too, and
// the reachability check reads that as "safely disabled". The namespace still
// exists and is not in MODULES, so nothing else fires, and a reachable family
// leaves the denominator without a word. "Not enabled" and "not a gate at all"
// must be different answers.
test('fails when an excluded namespace names a gate the manifest does not declare', (t) => {
  const root = fixture(t, { declareExcludedFeatures: false });
  write(root, 'src/openhuman/widgets/schemas_part_01.rs', controller('widgets', 'list'));
  write(root, 'tests/widgets_e2e.rs', 'let m = "openhuman.widgets_list";');

  const result = runGate(root);

  assert.equal(result.status, 1, `an undeclared gate must fail the gate; got:\n${result.stdout}`);
  assert.match(
    result.stderr,
    /name a feature the `\[features\]` table does not declare/,
    `the failure must distinguish a renamed gate from a disabled one; got:\n${result.stderr}`,
  );
  assert.match(
    result.stderr,
    /test \(gated on "e2e-test-support"\)/,
    `the offending entry must be named; got:\n${result.stderr}`,
  );
});

// CodeRabbit, PR #6092. TOML has two single-line string forms and cargo accepts
// both. Reading only `"…"` made `default = ['e2e-test-support']` parse as an
// empty array, so the gate saw the feature as OFF and accepted the exclusion
// for controllers the measured build actually compiles in.
test('reads a single-quoted TOML feature array', (t) => {
  const root = fixture(t);
  write(root, 'Cargo.toml', "[features]\ndefault = ['e2e-test-support']\ne2e-test-support = []\n");
  write(root, 'src/openhuman/widgets/schemas_part_01.rs', controller('widgets', 'list'));
  write(root, 'tests/widgets_e2e.rs', 'let m = "openhuman.widgets_list";');

  const result = runGate(root);

  assert.equal(result.status, 1, `a literal-string default list must still be read; got:\n${result.stdout}`);
  assert.match(
    result.stderr,
    /excluded namespace\(s\) are reachable in the measured configuration/,
    `a gate enabled through a literal string must count as enabled; got:\n${result.stderr}`,
  );
});

// Codex, PR #6092 (second round). The last hole: delete the `#[cfg]` in Rust
// and every other check here still passes — the feature is declared, it is
// disabled, the namespace is discovered, it is not in MODULES — while the
// module now compiles unconditionally and its controllers dispatch. An
// exclusion is a claim about the source, so it is checked against the source.
test('fails when the module is no longer behind the #[cfg] the exclusion claims', (t) => {
  const root = fixture(t, { excludedModuleCfg: null });
  write(root, 'src/openhuman/widgets/schemas_part_01.rs', controller('widgets', 'list'));
  write(root, 'tests/widgets_e2e.rs', 'let m = "openhuman.widgets_list";');

  const result = runGate(root);

  assert.equal(result.status, 1, `an ungated module must fail the gate; got:\n${result.stdout}`);
  assert.match(
    result.stderr,
    /no longer behind the gate they claim/,
    `the failure must say the #[cfg] is gone; got:\n${result.stderr}`,
  );
  assert.match(
    result.stderr,
    /src\/openhuman\/test_support\/schemas\.rs/,
    `the file that lost its gate must be named; got:\n${result.stderr}`,
  );
});

// The positive half, and the reason the walk climbs rather than reading the
// file itself: `#[cfg]` sits on the `mod` declaration in the PARENT. Here the
// gate is two levels up from the declaring file, with an ungated `mod schemas;`
// in between — the arrangement `src/openhuman/test_support/` actually has.
test('finds the gate on an ancestor mod declaration, not just the immediate parent', (t) => {
  const root = fixture(t);
  write(root, 'src/openhuman/widgets/schemas_part_01.rs', controller('widgets', 'list'));
  write(root, 'tests/widgets_e2e.rs', 'let m = "openhuman.widgets_list";');

  const result = runGate(root);

  assert.doesNotMatch(
    result.stderr,
    /no longer behind the gate they claim/,
    `a gate on a grandparent mod must count; got:\n${result.stderr}`,
  );
  assert.match(
    result.stdout,
    /Excluded 2 controller\(s\)/,
    `the exclusion must still apply; got:\n${result.stdout}`,
  );
});

// A namespace can declare its controllers in `mod.rs` itself rather than in a
// `schemas.rs` sibling. That file IS its directory's module, so its own `mod`
// declaration sits one level further up and under the DIRECTORY's name —
// resolving it like an ordinary file would look for `mod mod;` and find
// nothing, silently reporting the module as ungated.
test('resolves the gate for a namespace declared in mod.rs itself', (t) => {
  const root = fixture(t, { withExcludedNamespaces: false });
  write(
    root,
    'src/openhuman/test_support/mod.rs',
    controller('test', 'reset') + controller('test_support', 'workspace_root'),
  );
  write(root, 'src/openhuman/mod.rs', '#[cfg(feature = "e2e-test-support")]\npub mod test_support;\n');
  write(root, 'src/openhuman/widgets/schemas_part_01.rs', controller('widgets', 'list'));
  write(root, 'tests/widgets_e2e.rs', 'let m = "openhuman.widgets_list";');

  const result = runGate(root);

  assert.doesNotMatch(
    result.stderr,
    /no longer behind the gate they claim/,
    `a mod.rs-declared namespace must resolve its own gate; got:\n${result.stderr}`,
  );
  assert.match(
    result.stdout,
    /Excluded 2 controller\(s\)/,
    `the exclusion must still apply; got:\n${result.stdout}`,
  );
});

// Codex and CodeRabbit, PR #6092, independently. A `#[cfg]` that CONTAINS the
// feature name does not necessarily REQUIRE it, and the containing-the-name
// reading is wrong in the most dangerous direction — `not(feature = "x")`
// compiles precisely when the feature is OFF, so a namespace that is always
// present would have been accepted as always absent.
const NOT_A_GATE = [
  ['a negated feature', '#[cfg(not(feature = "e2e-test-support"))]'],
  ['an any() with an unrelated true branch', '#[cfg(any(feature = "e2e-test-support", unix))]'],
  ['an any() with another feature', '#[cfg(any(feature = "e2e-test-support", feature = "other"))]'],
  ['a negation nested in an all()', '#[cfg(all(unix, not(feature = "e2e-test-support")))]'],
];

for (const [description, attribute] of NOT_A_GATE) {
  test(`rejects ${description} as proof of the claimed gate`, (t) => {
    const root = fixture(t, { excludedModuleCfg: attribute });
    write(root, 'src/openhuman/widgets/schemas_part_01.rs', controller('widgets', 'list'));
    write(root, 'tests/widgets_e2e.rs', 'let m = "openhuman.widgets_list";');

    const result = runGate(root);

    assert.match(
      result.stderr,
      /no longer behind the gate they claim/,
      `${attribute} does not require the feature and must not count as a gate; got:\n${result.stderr}`,
    );
  });
}

// The other direction: a predicate whose truth genuinely implies the feature
// must still be accepted, or the check would fail every real gate that is not
// spelled in the simplest possible form.
const IS_A_GATE = [
  ['a bare feature', '#[cfg(feature = "e2e-test-support")]'],
  ['an all() alongside an unrelated term', '#[cfg(all(feature = "e2e-test-support", unix))]'],
  ['an any() where every branch requires it', '#[cfg(any(all(feature = "e2e-test-support", unix), all(feature = "e2e-test-support", windows)))]'],
  ['an attribute split across lines', '#[cfg(all(\n    feature = "e2e-test-support",\n    unix\n))]'],
  ['a cfg below a doc comment', '/// Gated.\n#[cfg(feature = "e2e-test-support")]'],
];

for (const [description, attribute] of IS_A_GATE) {
  test(`accepts ${description} as proof of the claimed gate`, (t) => {
    const root = fixture(t, { excludedModuleCfg: attribute });
    write(root, 'src/openhuman/widgets/schemas_part_01.rs', controller('widgets', 'list'));
    write(root, 'tests/widgets_e2e.rs', 'let m = "openhuman.widgets_list";');

    const result = runGate(root);

    assert.doesNotMatch(
      result.stderr,
      /no longer behind the gate they claim/,
      `${attribute} does require the feature and must count as a gate; got:\n${result.stderr}`,
    );
    assert.match(
      result.stdout,
      /Excluded 2 controller\(s\)/,
      `the exclusion must still apply; got:\n${result.stdout}`,
    );
  });
}

// Codex, PR #6092 (fourth round). A module can be declared more than once under
// mutually exclusive predicates, and it then exists in BOTH configurations:
//
//   #[cfg(feature = "x")]      mod test_support;
//   #[cfg(not(feature = "x"))] mod test_support;
//
// Reading only the first declaration sees the gate and calls the module absent
// when it is in fact always present.
test('fails when a second declaration makes the module reachable anyway', (t) => {
  const root = fixture(t, { withExcludedNamespaces: false });
  write(
    root,
    'src/openhuman/test_support/schemas.rs',
    controller('test', 'reset') + controller('test_support', 'workspace_root'),
  );
  write(root, 'src/openhuman/test_support/mod.rs', 'mod schemas;\n');
  write(
    root,
    'src/openhuman/mod.rs',
    '#[cfg(feature = "e2e-test-support")]\npub mod test_support;\n' +
      '#[cfg(not(feature = "e2e-test-support"))]\npub mod test_support;\n',
  );
  write(root, 'src/openhuman/widgets/schemas_part_01.rs', controller('widgets', 'list'));
  write(root, 'tests/widgets_e2e.rs', 'let m = "openhuman.widgets_list";');

  const result = runGate(root);

  assert.match(
    result.stderr,
    /no longer behind the gate they claim/,
    `a module declared in both configurations is always present; got:\n${result.stderr}`,
  );
});

// The same shape where both declarations DO require the feature stays accepted
// — otherwise the rule would reject every legitimate platform split.
test('accepts repeated declarations when every one of them requires the gate', (t) => {
  const root = fixture(t, { withExcludedNamespaces: false });
  write(
    root,
    'src/openhuman/test_support/schemas.rs',
    controller('test', 'reset') + controller('test_support', 'workspace_root'),
  );
  write(root, 'src/openhuman/test_support/mod.rs', 'mod schemas;\n');
  write(
    root,
    'src/openhuman/mod.rs',
    '#[cfg(all(feature = "e2e-test-support", unix))]\npub mod test_support;\n' +
      '#[cfg(all(feature = "e2e-test-support", windows))]\npub mod test_support;\n',
  );
  write(root, 'src/openhuman/widgets/schemas_part_01.rs', controller('widgets', 'list'));
  write(root, 'tests/widgets_e2e.rs', 'let m = "openhuman.widgets_list";');

  const result = runGate(root);

  assert.doesNotMatch(
    result.stderr,
    /no longer behind the gate they claim/,
    `a per-platform split that always requires the feature is still a gate; got:\n${result.stderr}`,
  );
  assert.match(
    result.stdout,
    /Excluded 2 controller\(s\)/,
    `the exclusion must still apply; got:\n${result.stdout}`,
  );
});
