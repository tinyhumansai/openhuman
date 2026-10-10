#!/usr/bin/env node
// Audit resolved normal/build edges, never Cargo.lock membership or dev edges.
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

export const CONTRACT_REGISTRY = 'registry+https://github.com/rust-lang/crates.io-index';

export const CONTRACT_CLOSURE = new Set([
  'serde', 'serde_core', 'serde_derive', 'serde_json', 'itoa', 'memchr', 'zmij',
  'thiserror', 'thiserror-impl', 'schemars', 'schemars_derive', 'dyn-clone',
  'ref-cast', 'ref-cast-impl', 'serde_derive_internals',
  'proc-macro2', 'quote', 'syn', 'unicode-ident',
]);

export function isImplementation(name, policy) {
  return policy.implementationPackages.includes(name) ||
    policy.implementationPrefixes.some(prefix => (name === prefix.replace(/-$/, '') || name.startsWith(prefix)) &&
      !policy.contracts.some(contract => contract.name === name));
}

/** Fail closed on unresolved metadata; retain package IDs (including versions). */
export function graph(metadata) {
  if (!metadata.resolve?.nodes || !metadata.packages?.length) {
    throw new Error('Cargo metadata must contain packages and a resolved dependency graph');
  }
  const packages = new Map(metadata.packages.map(pkg => [pkg.id, pkg]));
  const nodes = new Map(metadata.resolve.nodes.map(node => [node.id, node]));
  const edges = id => {
    const node = nodes.get(id);
    if (!packages.has(id) || !node) throw new Error(`missing resolved package ${id}`);
    if (!Array.isArray(node.deps)) throw new Error(`missing dependency kinds for ${id}`);
    return node.deps.filter(dep => {
      if (!Array.isArray(dep.dep_kinds) || dep.dep_kinds.length === 0) {
        throw new Error(`missing dependency kinds for ${dep.pkg}`);
      }
      if (dep.dep_kinds.some(kind => ![null, 'build', 'dev'].includes(kind.kind))) {
        throw new Error(`unknown dependency kind for ${dep.pkg}`);
      }
      return dep.dep_kinds.some(kind => kind.kind === null || kind.kind === 'build');
    }).map(dep => {
      if (!packages.has(dep.pkg) || !nodes.has(dep.pkg)) {
        throw new Error(`unresolved dependency ${dep.pkg}`);
      }
      return dep.pkg;
    });
  };
  return { packages, edges };
}

/** One shortest dependency path per forbidden package, including build scripts. */
export function auditGraph(metadata, roots, policy, contract = null) {
  const { packages, edges } = graph(metadata);
  const violations = [];
  const exceptions = [];
  for (const root of roots) {
    const candidates = [...packages.values()].filter(pkg => pkg.name === root);
    if (candidates.length !== 1) throw new Error(`expected exactly one root package ${root}`);
    const queue = [[candidates[0].id]];
    const seen = new Set();
    for (let at = 0; at < queue.length; at++) {
      const path = queue[at];
      const id = path.at(-1);
      if (seen.has(id)) continue;
      seen.add(id);
      const name = packages.get(id).name;
      if (path.length > 1) {
        const sourceMismatch = !contract && policy.contractManifests?.[name] &&
          packages.get(id).manifest_path !== policy.contractManifests[name];
        const forbidden = contract ? !CONTRACT_CLOSURE.has(name) || packages.get(id).source !== CONTRACT_REGISTRY :
          isImplementation(name, policy) || sourceMismatch;
        if (forbidden) {
          const scope = contract ?? 'hosts';
          const exemption = policy.exceptions.find(item => item.scope === scope && item.package === name);
          const finding = { scope, package: name, path: path.map(id => packages.get(id).name) };
          (exemption ? exceptions : violations).push(finding);
          // Contract exceptions pin the first unsafe dependency. Its implementation
          // closure is deliberately NOT approved vocabulary and must disappear
          // with this edge. Hosts still traverse it to catch indirect implementations.
          if (contract) continue;
        }
      }
      for (const next of edges(id)) queue.push([...path, next]);
    }
  }
  return { violations, exceptions };
}

export function validatePolicy(policy) {
  for (const key of ['implementationPackages', 'implementationPrefixes', 'contracts', 'pendingContracts', 'exceptions']) {
    if (!Array.isArray(policy[key])) throw new Error(`policy.${key} must be an array`);
  }
  if (!policy.contracts.length || !policy.implementationPackages.length || !policy.implementationPrefixes.length) {
    throw new Error('boundary inventory may not be empty');
  }
  const names = new Set();
  for (const contract of [...policy.contracts, ...policy.pendingContracts]) {
    if (!contract.name?.endsWith('-bus') || !contract.manifest || !contract.owner || names.has(contract.name)) {
      throw new Error(`invalid or duplicate contract ${contract.name}`);
    }
    if (policy.pendingContracts.includes(contract) && !contract.reason?.trim()) {
      throw new Error(`pending contract ${contract.name} needs a reason`);
    }
    names.add(contract.name);
  }
  const seen = new Set();
  for (const item of policy.exceptions) {
    const key = `${item.scope}:${item.package}`;
    if (!item.reason?.trim() || !item.package || !item.scope || seen.has(key)) {
      throw new Error(`invalid or duplicate exception ${key}`);
    }
    if (item.scope === 'hosts' ? !isImplementation(item.package, policy) :
      !names.has(item.scope) || CONTRACT_CLOSURE.has(item.package)) {
      throw new Error(`exception ${key} does not name a forbidden dependency`);
    }
    seen.add(key);
  }
}

function cargo(root, args) {
  // All generated manifests and logs stay under the checkout's target/.
  const stdout = execFileSync('bash', [join(root, 'scripts/ci-cancel-aware.sh'),
    'cargo', 'metadata', '--format-version', '1', ...args], {
    cwd: root, encoding: 'utf8', maxBuffer: 128 * 1024 * 1024, stdio: ['ignore', 'pipe', 'inherit'],
  });
  return JSON.parse(stdout);
}

/** Resolve only this contract's selected features, without module/host unification. */
function contractMetadata(root, contract, allFeatures) {
  const manifest = resolve(root, contract.manifest);
  const listing = cargo(root, ['--manifest-path', manifest, '--no-deps']);
  const pkg = listing.packages.find(pkg => resolve(pkg.manifest_path) === manifest);
  if (!pkg || pkg.name !== contract.name) throw new Error(`missing contract ${contract.name}`);
  const harness = join(root, 'target/module-boundaries', contract.name, allFeatures ? 'all' : 'default');
  mkdirSync(join(harness, 'src'), { recursive: true });
  const features = allFeatures ? Object.keys(pkg.features) : [];
  writeFileSync(join(harness, 'Cargo.toml'), `[workspace]\n[package]\nname = "boundary-probe"\nversion = "0.0.0"\nedition = "2024"\n[dependencies]\n${contract.name} = { path = ${JSON.stringify(dirname(manifest))}, features = ${JSON.stringify(features)} }\n`);
  writeFileSync(join(harness, 'src/lib.rs'), '');
  return cargo(root, ['--manifest-path', join(harness, 'Cargo.toml')]);
}

export function run(root, strict = false) {
  const policy = JSON.parse(readFileSync(join(root, 'scripts/ci/module-boundaries.json'), 'utf8'));
  validatePolicy(policy);
  policy.contractManifests = Object.fromEntries(policy.contracts.map(contract =>
    [contract.name, resolve(root, contract.manifest)]));
  for (const contract of policy.pendingContracts) {
    if (existsSync(join(root, contract.manifest))) {
      throw new Error(`pending contract ${contract.name} now exists: register it for independent auditing`);
    }
  }
  const results = [];
  for (const [manifest, roots] of [
    ['Cargo.toml', ['openhuman', 'openhuman-cli', 'openhuman-tui']],
    ['crates/openhuman-app/Cargo.toml', ['openhuman-app']],
  ]) {
    const metadata = cargo(root, ['--manifest-path', join(root, manifest), '--locked', '--all-features']);
    results.push(auditGraph(metadata, roots, policy));
  }
  for (const contract of policy.contracts) {
    for (const all of [false, true]) {
      results.push(auditGraph(contractMetadata(root, contract, all), [contract.name], policy, contract.name));
    }
  }
  const violations = results.flatMap(result => result.violations);
  const exceptions = results.flatMap(result => result.exceptions);
  for (const item of violations) console.error(`FORBIDDEN [${item.scope}] ${item.path.join(' -> ')}`);
  const used = new Set(exceptions.map(item => `${item.scope}:${item.package}`));
  const stale = policy.exceptions.filter(item => !used.has(`${item.scope}:${item.package}`));
  for (const item of stale) console.error(`STALE exception [${item.scope}] ${item.package}: remove it`);
  const pending = policy.pendingContracts.length + policy.exceptions.length;
  console.log(`Module boundaries: ${violations.length} violations, ${stale.length} stale exceptions; ${policy.exceptions.length} temporary exceptions, ${policy.pendingContracts.length} pending contracts.`);
  if (pending) console.log('Migration INCOMPLETE. --require-complete rejects every temporary exception and pending contract.');
  return violations.length === 0 && stale.length === 0 && (!strict || pending === 0);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const args = process.argv.slice(2);
    if (args.some(arg => arg.startsWith('--') && arg !== '--require-complete')) throw new Error('unknown option');
    const root = resolve(args.find(arg => !arg.startsWith('--')) ?? join(dirname(fileURLToPath(import.meta.url)), '../..'));
    process.exitCode = run(root, args.includes('--require-complete')) ? 0 : 1;
  } catch (error) {
    console.error(`Module boundary audit could not complete: ${error.message}`);
    process.exitCode = 2;
  }
}
