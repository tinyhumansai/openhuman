import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { auditGraph, graph, isImplementation, validatePolicy, CONTRACT_REGISTRY } from '../ci/check-module-boundaries.mjs';

const policy = JSON.parse(readFileSync(new URL('../ci/module-boundaries.json', import.meta.url), 'utf8'));
const strict = { ...policy, exceptions: [] };
function metadata(edges, names = {}) {
  const ids = [...new Set(Object.entries(edges).flatMap(([id, deps]) => [id, ...deps.map(d => d[0])]))];
  return {
    packages: ids.map(id => ({ id, name: names[id] ?? id, source: CONTRACT_REGISTRY })),
    resolve: { nodes: ids.map(id => ({ id, deps: (edges[id] ?? []).map(([pkg, kind = null, target = null]) => ({ pkg, dep_kinds: [{ kind, target }] })) })) },
  };
}

test('checked-in inventory has explicit owners and reasoned exceptions', () => validatePolicy(policy));
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
  assert.equal(auditGraph(m, ['tinyjuice-bus'], policy, 'tinyjuice-bus').violations.length, 1);
});
test('exceptions retain findings and do not hide new host descendants', () => {
  const m = metadata({ host: [['tinyjuice']], tinyjuice: [['tinyjuice-new-engine']] });
  const result = auditGraph(m, ['host'], policy);
  assert.equal(result.exceptions.length, 1);
  assert.equal(result.violations[0].package, 'tinyjuice-new-engine');
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
