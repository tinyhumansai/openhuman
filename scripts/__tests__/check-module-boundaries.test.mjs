import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import test from 'node:test';
import { auditGraph, bridgeReach, graph, isImplementation, linkedHostAudit, run, treeGraph, validatePolicy, CONTRACT_REGISTRY } from '../ci/check-module-boundaries.mjs';

const policy = JSON.parse(readFileSync(new URL('../ci/module-boundaries.json', import.meta.url), 'utf8'));
const strict = { ...policy, exceptions: [] };
const fixtureException = { ...strict, exceptions: [{ scope: 'hosts', package: 'tinyjuice', reason: 'Explicit test fixture for exception semantics' }] };
function metadata(edges, names = {}) {
  const ids = [...new Set(Object.entries(edges).flatMap(([id, deps]) => [id, ...deps.map(d => d[0])]))];
  return {
    packages: ids.map(id => ({ id, name: names[id] ?? id, source: CONTRACT_REGISTRY })),
    resolve: { nodes: ids.map(id => ({ id, deps: (edges[id] ?? []).map(([pkg, kind = null, target = null]) => ({ pkg, dep_kinds: [{ kind, target }] })) })) },
  };
}

test('checked-in inventory has explicit owners and reasoned exceptions', () => validatePolicy(policy));
test('TinyJuice host adapter is allowed while implementation dependencies remain audited', () => {
  assert.equal(isImplementation('tinyjuice-host', policy), false);
  assert.equal(isImplementation('tinyjuice-core', policy), true);
  const m = metadata({ host: [['tinyjuice-host']], 'tinyjuice-host': [['tinyjuice']] });
  assert.deepEqual(auditGraph(m, ['host'], policy).violations.map(v => v.path), [
    ['host', 'tinyjuice-host', 'tinyjuice'],
  ]);
});
test('host traversal rejects direct, wrapper and build dependencies', () => {
  const m = metadata({ host: [['wrapper'], ['tinyvoice', 'build']], wrapper: [['tinyjuice']] });
  const result = auditGraph(m, ['host'], strict);
  assert.deepEqual(result.violations.map(v => v.path), [['host', 'tinyvoice'], ['host', 'wrapper', 'tinyjuice']]);
});
test('dev-only implementation dependencies do not enter shipped graphs', () => {
  const m = metadata({ host: [['test-helper', 'dev']], 'test-helper': [['tinyjuice']] });
  assert.deepEqual(auditGraph(m, ['host'], strict).violations, []);
});
test('a dependency with both dev and normal kinds remains forbidden', () => {
  const m = metadata({ host: [['tinyjuice', 'dev']] });
  m.resolve.nodes[0].deps[0].dep_kinds.push({ kind: null, target: null });
  assert.equal(auditGraph(m, ['host'], strict).violations.length, 1);
});
test('platform-specific build and normal dependencies are inspected', () => {
  const m = metadata({ host: [['tinybox-jail', null, 'cfg(windows)'], ['tinyvoice', 'build', 'cfg(target_os = "macos")']] });
  assert.equal(auditGraph(m, ['host'], strict).violations.length, 2);
});
test('package identity is not a dependency alias', () => {
  const m = metadata({ host: [['pkg#implementation']] }, { 'pkg#implementation': 'tinyjuice' });
  assert.equal(auditGraph(m, ['host'], strict).violations[0].package, 'tinyjuice');
});
test('new implementation packages in owning repositories are forbidden', () => {
  assert.equal(isImplementation('tinywallet-new-provider', strict), true);
  assert.equal(isImplementation('tinywallet-bus', strict), false);
  assert.equal(isImplementation('tinywallet-unregistered-bus', strict), true);
  assert.equal(isImplementation('tinysearch', strict), true);
});
test('contracts permit only the serialization/schema/error closure', () => {
  const m = metadata({ 'tinyjuice-bus': [['serde'], ['thiserror']], serde: [['serde_derive']], serde_derive: [['syn']], thiserror: [['thiserror-impl']] });
  assert.deepEqual(auditGraph(m, ['tinyjuice-bus'], strict, 'tinyjuice-bus').violations, []);
});
test('contracts allow pinned canonical TSD vocabulary and reject other git sources', () => {
  const m = metadata({ 'tinyjuice-bus': [['tinystoragedrivers-types']] });
  const types = m.packages.find(pkg => pkg.name === 'tinystoragedrivers-types');
  const revision = 'a'.repeat(40);
  types.source = `git+https://github.com/tinyhumansai/tinystoragedrivers?rev=${revision}#${revision.slice(0, 8)}`;
  assert.deepEqual(auditGraph(m, ['tinyjuice-bus'], strict, 'tinyjuice-bus').violations, []);

  types.source = `git+https://github.com/senamakel/tinystoragedrivers?rev=${revision}#${revision}`;
  assert.equal(auditGraph(m, ['tinyjuice-bus'], strict, 'tinyjuice-bus').violations[0].package, 'tinystoragedrivers-types');
  types.source = 'git+https://github.com/tinyhumansai/tinystoragedrivers#main';
  assert.equal(auditGraph(m, ['tinyjuice-bus'], strict, 'tinyjuice-bus').violations[0].package, 'tinystoragedrivers-types');
});
test('contracts reject transport, runtime, HTTP, database and native libraries', () => {
  for (const dependency of ['tinybus', 'tokio', 'reqwest', 'rusqlite', 'cpal', 'tinyjuice']) {
    const m = metadata({ 'tinyjuice-bus': [[dependency]] });
    assert.equal(auditGraph(m, ['tinyjuice-bus'], strict, 'tinyjuice-bus').violations[0].package, dependency);
  }
});
test('contracts reject an indirect implementation in an approved dependency', () => {
  const m = metadata({ 'tinyjuice-bus': [['serde']], serde: [['tinyjuice']] });
  assert.deepEqual(auditGraph(m, ['tinyjuice-bus'], strict, 'tinyjuice-bus').violations[0].path, ['tinyjuice-bus', 'serde', 'tinyjuice']);
});
test('host exceptions do not exempt contract dependencies', () => {
  const m = metadata({ 'tinyjuice-bus': [['tinyjuice']] });
  assert.equal(auditGraph(m, ['tinyjuice-bus'], fixtureException, 'tinyjuice-bus').violations.length, 1);
});
test('exceptions retain findings and do not hide new host descendants', () => {
  const m = metadata({ host: [['tinyjuice']], tinyjuice: [['tinyjuice-new-engine']] });
  const result = auditGraph(m, ['host'], fixtureException);
  assert.equal(result.exceptions.length, 1);
  assert.equal(result.violations[0].package, 'tinyjuice-new-engine');
});
test('parent-scoped host exceptions do not allow direct implementation dependencies', () => {
  const m = metadata({ openhuman: [['tinyflows']], cli: [['tinyflows'], ['openhuman']] });
  const direct = auditGraph(m, ['cli'], policy);
  assert.deepEqual(direct.violations.map(item => item.path), [['cli', 'tinyflows']]);
  const core = auditGraph(m, ['openhuman'], policy);
  assert.deepEqual(core.exceptions.map(item => item.path), [['openhuman', 'tinyflows']]);
});
test('parent-scoped exceptions are evaluated for every incoming edge', () => {
  const edgePolicy = {
    ...strict,
    implementationPackages: ['tinyflows'],
    implementationPrefixes: [],
    exceptions: [{ scope: 'hosts', package: 'tinyflows', parent: 'tinyflows-import', reason: 'import bridge' }],
  };
  const m = metadata({
    openhuman: [['tinyflows'], ['tinyflows-import']],
    'tinyflows-import': [['tinyflows']],
  });
  const result = auditGraph(m, ['openhuman'], edgePolicy);
  assert.deepEqual(result.violations.map(item => item.path), [['openhuman', 'tinyflows']]);
  assert.deepEqual(result.exceptions.map(item => item.path), [
    ['openhuman', 'tinyflows-import', 'tinyflows'],
  ]);
});
test('cycles terminate and versions preserve distinct package identities', () => {
  const m = metadata({ host: [['juice-old'], ['juice-new']], 'juice-old': [['host']] }, { 'juice-old': 'tinyjuice', 'juice-new': 'tinyjuice' });
  assert.equal(auditGraph(m, ['host'], strict).violations.length, 2);
});
test('missing resolve, missing roots and dangling edges fail closed', () => {
  assert.throws(() => graph({ packages: [] }), /resolved dependency graph/);
  assert.throws(() => auditGraph(metadata({ host: [] }), ['missing'], strict), /root package/);
  const m = metadata({ host: [['serde']] });
  m.resolve.nodes.pop();
  assert.throws(() => auditGraph(m, ['host'], strict), /unresolved dependency/);
});
test('missing dependency kinds fail closed rather than assuming dev-only', () => {
  const m = metadata({ host: [['tinyjuice']] });
  m.resolve.nodes[0].deps[0].dep_kinds = [];
  assert.throws(() => auditGraph(m, ['host'], strict), /missing dependency kinds/);
});
test('inventory rejects broad, duplicate, empty-reason or mis-scoped exceptions', () => {
  for (const item of [
    { scope: 'hosts', package: '*', reason: 'broad' },
    { scope: 'hosts', package: 'tinyjuice', reason: '' },
    { scope: 'tinyjuice-bus', package: 'serde', reason: 'approved' },
    { scope: 'missing-bus', package: 'tokio', reason: 'untracked' },
    policy.exceptions[0],
  ]) assert.throws(() => validatePolicy({ ...policy, exceptions: [...policy.exceptions, item] }));
});

test('a same-named contract from another source cannot evade independent auditing', () => {
  const m = metadata({ host: [['tinyjuice-bus']] });
  m.packages.find(pkg => pkg.name === 'tinyjuice-bus').manifest_path = '/unexpected/Cargo.toml';
  const pinned = { ...strict, contractManifests: { 'tinyjuice-bus': '/pinned/Cargo.toml' } };
  assert.equal(auditGraph(m, ['host'], pinned).violations.length, 1);
  m.packages.find(pkg => pkg.name === 'tinyjuice-bus').manifest_path = '/pinned/Cargo.toml';
  assert.equal(auditGraph(m, ['host'], pinned).violations.length, 0);
});


test('allowlisted vocabulary names cannot mask a path, git or alternate-registry package', () => {
  for (const source of [null, undefined, 'git+https://example.invalid/serde', 'registry+https://example.invalid/index']) {
    const m = metadata({ 'tinyjuice-bus': [['serde']], serde: [['syn']] });
    m.packages.find(pkg => pkg.name === 'serde').source = source;
    const result = auditGraph(m, ['tinyjuice-bus'], strict, 'tinyjuice-bus');
    assert.deepEqual(result.violations.map(v => v.package), ['serde']);
  }
});

test('every approved vocabulary descendant must come from the approved registry', () => {
  const m = metadata({ 'tinyjuice-bus': [['serde']], serde: [['syn']] });
  m.packages.find(pkg => pkg.name === 'syn').source = null;
  assert.deepEqual(auditGraph(m, ['tinyjuice-bus'], strict, 'tinyjuice-bus').violations[0].path, ['tinyjuice-bus', 'serde', 'syn']);
});

test('bare owning packages and their implementation descendants are both forbidden', () => {
  for (const prefix of policy.implementationPrefixes) {
    const owner = prefix.replace(/-$/, '');
    assert.equal(isImplementation(owner, strict), true, owner);
    assert.equal(isImplementation(`${owner}-new-engine`, strict), true, owner);
    const contract = policy.contracts.find(item => item.name === `${owner}-bus`);
    if (contract) assert.equal(isImplementation(contract.name, strict), false, contract.name);
  }
});


test('the separately removed runtime is outside this migration but cannot enter another contract', () => {
  const excluded = policy.excludedSubsystems?.find(item => item.owner === 'tinyruntime');
  assert.ok(excluded?.reason?.trim());
  const host = metadata({ host: [['tinyruntime-pyserver'], ['tinyjuice']] });
  assert.deepEqual(auditGraph(host, ['host'], strict).violations.map(item => item.package), ['tinyjuice']);
  assert.ok(!policy.exceptions.some(item => item.package.startsWith('tinyruntime')));
  const contract = metadata({ 'tinyjuice-bus': [['tinyruntime-bus']] });
  assert.equal(auditGraph(contract, ['tinyjuice-bus'], strict, 'tinyjuice-bus').violations[0].package, 'tinyruntime-bus');
});

test('the migrated TinyJuice implementation has no host exception', () => {
  assert.equal(policy.exceptions.some(item => item.scope === 'hosts' && item.package === 'tinyjuice'), false);
  const m = metadata({ host: [['tinyjuice']] });
  assert.equal(auditGraph(m, ['host'], policy).violations[0].package, 'tinyjuice');
});

test('contract binary payload serialization permits base64 without admitting crypto or transport', () => {
  const m = metadata({ 'tinychannels-bus': [['serde'], ['base64']] });
  assert.deepEqual(auditGraph(m, ['tinychannels-bus'], strict, 'tinychannels-bus').violations, []);
  for (const dependency of ['hmac', 'sha2', 'tokio', 'tinychannels-runtime']) {
    const invalid = metadata({ 'tinychannels-bus': [['base64']], base64: [[dependency]] });
    assert.deepEqual(auditGraph(invalid, ['tinychannels-bus'], strict, 'tinychannels-bus').violations.map(v => v.package), [dependency]);
  }
});

test('the linked-modules bridge is allowed directly under the CLI only, without auditing its subtree', () => {
  const linked = { ...strict, linkedHost: { package: 'openhuman-modules-linked', allowedParents: ['openhuman-cli'] } };
  const ok = metadata({ 'openhuman-cli': [['openhuman-modules-linked']], 'openhuman-modules-linked': [['tinydocs-module']], 'tinydocs-module': [['tinydocs']] });
  assert.deepEqual(auditGraph(ok, ['openhuman-cli'], linked).violations, []);
  assert.equal(policy.linkedHost.package, 'openhuman-modules-linked');
  assert.deepEqual(policy.linkedHost.allowedParents, ['openhuman-cli']);
});

test('a legal direct bridge edge does not hide a second path through a wrapper', () => {
  const linked = { ...strict, linkedHost: { package: 'openhuman-modules-linked', allowedParents: ['openhuman-cli'] } };
  const m = metadata({
    'openhuman-cli': [['openhuman-modules-linked'], ['wrapper']],
    wrapper: [['openhuman-modules-linked']],
    'openhuman-modules-linked': [['tinydocs-module']],
  });
  assert.deepEqual(
    auditGraph(m, ['openhuman-cli'], linked).violations.map(v => v.path),
    [['openhuman-cli', 'wrapper', 'openhuman-modules-linked']],
  );
  assert.deepEqual(
    linkedHostAudit(m, 'openhuman-cli', linked).violations.map(v => v.path),
    [['openhuman-cli', 'wrapper', 'openhuman-modules-linked']],
  );
});

test('linked bridge closure does not excuse an unrelated implementation edge or bus source mismatch', () => {
  const linked = {
    ...strict,
    linkedHost: { package: 'openhuman-modules-linked', allowedParents: ['openhuman-cli'] },
    contractManifests: { 'tinyflows-bus': '/canonical/tinyflows-bus/Cargo.toml' },
  };
  const m = metadata({
    'openhuman-cli': [['openhuman-modules-linked'], ['openhuman'], ['tinyflows']],
    'openhuman-modules-linked': [['tinyflows-import']],
    openhuman: [['tinyflows-import']],
    'tinyflows-import': [['tinyflows'], ['tinyflows-bus']],
  });
  const findings = linkedHostAudit(m, 'openhuman-cli', linked).violations;
  assert.deepEqual(findings.map(v => [v.package, v.path]), [
    ['tinyflows', ['openhuman-cli', 'tinyflows']],
    ['tinyflows-bus', ['openhuman-cli', 'openhuman', 'tinyflows-import', 'tinyflows-bus']],
  ]);
  assert.throws(() => linkedHostAudit(metadata({ 'openhuman-cli': [] }), 'openhuman-cli', linked), /does not contain openhuman-modules-linked/);
});

test('deliberate violation: the bridge reached from the app, the TUI, the core or through a wrapper fails', () => {
  const linked = { ...strict, linkedHost: { package: 'openhuman-modules-linked', allowedParents: ['openhuman-cli'] } };
  for (const host of ['openhuman-app', 'openhuman-tui', 'openhuman']) {
    const m = metadata({ [host]: [['openhuman-modules-linked']], 'openhuman-modules-linked': [['tinydocs-module']] });
    const result = auditGraph(m, [host], linked);
    assert.deepEqual(result.violations.map(v => v.path), [[host, 'openhuman-modules-linked']], host);
  }
  const wrapped = metadata({ 'openhuman-cli': [['wrapper']], wrapper: [['openhuman-modules-linked']] });
  assert.equal(auditGraph(wrapped, ['openhuman-cli'], linked).violations.length, 1);
});

test('without a linkedHost policy the bridge subtree is audited like any other dependency', () => {
  const m = metadata({ 'openhuman-cli': [['openhuman-modules-linked']], 'openhuman-modules-linked': [['tinydocs-module']] });
  assert.deepEqual(auditGraph(m, ['openhuman-cli'], { ...strict, linkedHost: undefined }).violations.map(v => v.package), ['tinydocs-module']);
});

test('linkedHost policy validation rejects an empty parent list and an implementation package', () => {
  assert.throws(() => validatePolicy({ ...policy, linkedHost: { package: 'x', allowedParents: [] } }), /linkedHost/);
  assert.throws(() => validatePolicy({ ...policy, linkedHost: { package: 'tinydocs-module', allowedParents: ['openhuman-cli'] } }), /implementation/);
});

test('without linked-modules no host reaches the bridge; with it, the CLI does', () => {
  const without = metadata({ 'openhuman-cli': [['openhuman-rpc']], 'openhuman-tui': [['openhuman-rpc']], openhuman: [] });
  assert.deepEqual(bridgeReach(without, ['openhuman-cli', 'openhuman-tui', 'openhuman'], policy), []);
  const withIt = metadata({
    'openhuman-cli': [['openhuman-rpc'], ['openhuman-modules-linked']],
    'openhuman-tui': [['openhuman-rpc']],
    openhuman: [],
  });
  assert.deepEqual(
    bridgeReach(withIt, ['openhuman-cli', 'openhuman-tui', 'openhuman'], policy).map(v => v.path),
    [['openhuman-cli', 'openhuman-modules-linked']]
  );
  // A dev edge is not a shipped edge.
  const dev = metadata({ 'openhuman-cli': [['openhuman-modules-linked', 'dev']], 'openhuman-modules-linked': [] });
  assert.deepEqual(bridgeReach(dev, ['openhuman-cli'], policy), []);
});

test('Cargo tree parsing preserves package versions, path identity, and dedup edges', () => {
  const localBus = '/checkout/vendor/tinyjuice/crates/tinyjuice-bus';
  const m = treeGraph([
    '0openhuman v0.64.18 (/checkout/crates/openhuman-core)',
    `1tinyjuice-bus v0.1.0 (${localBus})`,
    '1wrapper v1.0.0',
    '2shared v1.0.0',
    '1wrapper v1.0.0 (*)',
    '1wrapper v1.0.0',
    '2tinyjuice v1.0.0',
    '2tinyjuice v2.0.0',
  ].join('\n'));
  const bus = m.packages.find(pkg => pkg.name === 'tinyjuice-bus');
  assert.equal(bus.source, null);
  assert.equal(bus.manifest_path, `${localBus}/Cargo.toml`);
  assert.equal(m.packages.filter(pkg => pkg.name === 'tinyjuice').length, 2);
  assert.equal(m.resolve.nodes.find(node => node.id.startsWith('tree:2:wrapper v1.0.0')).deps.length, 3);
  const pinned = {
    ...strict,
    contractManifests: { 'tinyjuice-bus': `${localBus}/Cargo.toml` },
  };
  assert.equal(auditGraph(m, ['openhuman'], strict).violations.length, 2);
  const pinnedTree = treeGraph([
    '0openhuman v0.64.18',
    `1tinyjuice-bus v0.1.0 (${localBus})`,
  ].join('\n'));
  assert.deepEqual(auditGraph(pinnedTree, ['openhuman'], pinned).violations, []);
  const unpinned = treeGraph([
    '0openhuman v0.64.18',
    '1tinyjuice-bus v0.1.0',
  ].join('\n'));
  assert.equal(auditGraph(unpinned, ['openhuman'], pinned).violations[0].package, 'tinyjuice-bus');
});

test('a package-scoped normal tree still rejects an activated implementation writer', () => {
  const m = treeGraph([
    '0openhuman v0.64.18',
    '1tinyflows-import v0.1.0',
    '2tinyflows v0.8.28',
    '2tinyflows-catalog v0.1.0',
    '2tinyflows-drivers v0.1.0',
  ].join('\n'));
  assert.deepEqual(
    auditGraph(m, ['openhuman'], strict).violations.map(item => item.package),
    ['tinyflows', 'tinyflows-catalog', 'tinyflows-drivers'],
  );
});

test('host boundary audit ignores writer features unified only by unrelated workspace dev fixtures', () => {
  const root = resolve(new URL('../..', import.meta.url).pathname);
  mkdirSync(join(root, 'target'), { recursive: true });
  const harnessRoot = mkdtempSync(join(root, 'target', 'module-boundaries-host-isolation-'));
  try {
    mkdirSync(join(harnessRoot, 'scripts', 'ci'), { recursive: true });
    writeFileSync(join(harnessRoot, 'scripts', 'ci', 'module-boundaries.json'), JSON.stringify(policy));

    // Workspace-wide metadata sees an opt-in test writer on tinyflows-import
    // as part of openhuman's unified normal feature graph. The scoped core tree
    // also carries its explicitly tracked TinyFlows compatibility dependency.
    const unified = metadata({
      openhuman: [['tinyflows-import'], ['tinyflows']],
      'openhuman-cli': [['openhuman-rpc']],
      'openhuman-tui': [['openhuman-rpc']],
      'openhuman-rpc': [['openhuman-tinyhumans']],
      'openhuman-tinyhumans': [['openhuman-embed']],
      'openhuman-embed': [['openhuman']],
      'tinyflows-import': [
        ['tinyflows'],
        ['tinyflows-catalog'],
        ['tinyflows-drivers'],
        ['tinyflows-schedule'],
      ],
    });
    const scoped = metadata({
      openhuman: [['tinyflows-import'], ['tinyflows']],
      'tinyflows-import': [['tinyflows-bus']],
    });
    const app = metadata({
      'openhuman-app': [['openhuman-rpc']],
      'openhuman-rpc': [['openhuman-tinyhumans']],
      'openhuman-tinyhumans': [['openhuman-embed']],
      'openhuman-embed': [['openhuman']],
      openhuman: [['tinyflows-import'], ['tinyflows']],
      'tinyflows-import': [
        ['tinyflows'],
        ['tinyflows-catalog'],
        ['tinyflows-drivers'],
        ['tinyflows-schedule'],
      ],
    });
    const defaults = metadata({
      'openhuman-cli': [['openhuman-rpc']],
      'openhuman-tui': [['openhuman-rpc']],
      openhuman: [],
    });
    assert.equal(auditGraph(unified, ['openhuman'], strict).violations.length, 4);
    assert.deepEqual(auditGraph(scoped, ['openhuman'], policy).violations, []);
    assert.deepEqual(auditGraph(scoped, ['openhuman'], policy).exceptions.map(item => item.package), ['tinyflows']);
    const manifestToContract = new Map(policy.contracts.map(item => [
      resolve(harnessRoot, item.manifest), item.name,
    ]));
    let contractBeingProbed;
    let defaultWriterEnabled = false;

    const fakeExecFileSync = (_file, args) => {
      assert.equal(args[1], 'cargo');
      const cargoArgs = args.slice(2);
      const manifestPath = cargoArgs[cargoArgs.indexOf('--manifest-path') + 1];
      if (cargoArgs[0] === 'tree') {
        const packageName = cargoArgs[cargoArgs.indexOf('-p') + 1];
        assert.ok(packageName, 'every host tree must be rooted at a package, including binaries');
        const links = packageName === 'openhuman-cli' && cargoArgs.includes('--features') &&
          cargoArgs[cargoArgs.indexOf('--features') + 1].split(',').includes('linked-modules');
        if (packageName === 'openhuman-cli' && !links && cargoArgs.includes('--features')) {
          const selected = cargoArgs[cargoArgs.indexOf('--features') + 1].split(',');
          assert.ok(!cargoArgs.includes('--no-default-features'), 'CLI default features remain enabled');
          assert.ok(selected.includes('flow-write'), 'all non-bridge CLI features remain covered');
          assert.ok(!selected.includes('linked-modules'), 'linked-modules is isolated from the shipping graph');
        }
        const tinyflowsBus = resolve(harnessRoot, 'vendor/tinyflows/crates/tinyflows-bus');
        return [
          `0${packageName} v0.0.0`,
          ...(links ? [
            '1openhuman-modules-linked v0.0.0',
            '2tinydocs-module v0.0.0',
            '3tinydocs v0.0.0',
          ] : packageName === 'openhuman-cli' && defaultWriterEnabled && cargoArgs.includes('--features') ? [
            '1tinyflows v0.8.28',
          ] : packageName === 'openhuman' ? [
            '1tinyflows v0.8.28',
            '1tinyflows-import v0.1.0',
            `2tinyflows-bus v0.1.0 (${tinyflowsBus})`,
          ] : []),
        ].join('\n');
      }
      if (cargoArgs.includes('--no-deps')) {
        if (resolve(manifestPath) === resolve(harnessRoot, 'Cargo.toml')) {
          return JSON.stringify({
            packages: [{
              name: 'openhuman-cli',
              manifest_path: resolve(harnessRoot, 'crates/openhuman-cli/Cargo.toml'),
              features: {
                default: ['read'],
                read: [],
                'linked-modules': [],
                'flow-write': [],
              },
            }],
          });
        }
        contractBeingProbed = manifestToContract.get(resolve(manifestPath));
        assert.ok(contractBeingProbed, `unexpected manifest ${manifestPath}`);
        return JSON.stringify({
          packages: [{
            id: `${contractBeingProbed} 0.0.0`,
            name: contractBeingProbed,
            manifest_path: manifestPath,
            features: {},
          }],
        });
      }
      if (manifestPath.includes('target/module-boundaries/')) {
        const exceptionPackages = policy.exceptions
          .filter(item => item.scope === contractBeingProbed)
          .map(item => [item.package]);
        const contractMetadata = metadata({
          [contractBeingProbed]: exceptionPackages,
        });
        return JSON.stringify(contractMetadata);
      }
      if (manifestPath.endsWith('crates/openhuman-app/Cargo.toml')) {
        return JSON.stringify(app);
      }
      return JSON.stringify(cargoArgs.includes('--all-features') ? unified : defaults);
    };

    // A scoped host graph should pass. The existing workspace metadata probe
    // instead observes the unrelated dev fixture's flow-write feature and
    // reports implementation packages along the normal host path.
    assert.equal(run(harnessRoot, false, fakeExecFileSync), true);
    // Model an optional dependency activated only by the CLI's default feature.
    // It must appear in the all-shipping-feature graph and be rejected.
    defaultWriterEnabled = true;
    assert.equal(run(harnessRoot, false, fakeExecFileSync), false);
  } finally {
    rmSync(harnessRoot, { recursive: true, force: true });
  }
});
