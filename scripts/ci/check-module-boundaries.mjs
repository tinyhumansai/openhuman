#!/usr/bin/env node
// Audit resolved normal/build edges, never Cargo.lock membership or dev edges.
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

export const CONTRACT_REGISTRY = 'registry+https://github.com/rust-lang/crates.io-index';
const CONTRACT_GIT_SOURCES = new Map([
  ['tinystoragedrivers-types', 'https://github.com/tinyhumansai/tinystoragedrivers'],
]);

function contractSourceAllowed(name, source) {
  if (source === CONTRACT_REGISTRY) return true;
  const repository = CONTRACT_GIT_SOURCES.get(name);
  if (!repository || typeof source !== 'string') return false;
  const prefix = `git+${repository}?rev=`;
  if (!source.startsWith(prefix)) return false;
  const [revision, resolved] = source.slice(prefix.length).split('#');
  return /^[0-9a-f]{40}$/.test(revision) &&
    typeof resolved === 'string' && resolved.length >= 8 && revision.startsWith(resolved);
}

export const CONTRACT_CLOSURE = new Set([
  'serde', 'serde_core', 'serde_derive', 'serde_json', 'itoa', 'memchr', 'zmij',
  'thiserror', 'thiserror-impl', 'schemars', 'schemars_derive', 'dyn-clone',
  'ref-cast', 'ref-cast-impl', 'serde_derive_internals',
  'proc-macro2', 'quote', 'syn', 'unicode-ident',
  // Base64 encoding of serialized binary contract fields; no transport/crypto.
  'base64',
  // TSD v2's data-only storage contract vocabulary (Scope, specs, errors).
  'tinystoragedrivers-types',
]);

export function isImplementation(name, policy) {
  if (policy.hostAdapterPackages?.includes(name)) return false;
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
  const reportedEdges = new Set();
  for (const root of roots) {
    const candidates = [...packages.values()].filter(pkg => pkg.name === root);
    if (candidates.length !== 1) throw new Error(`expected exactly one root package ${root}`);
    const queue = [[candidates[0].id]];
    const seen = new Set();
    for (let at = 0; at < queue.length; at++) {
      const path = queue[at];
      const id = path.at(-1);
      const name = packages.get(id).name;
      // The one crate that may link module implementations (local development
      // only). It is a legitimate edge only directly under an allowed host
      // crate, and its own subtree is not audited: that is the point of it. Any
      // other route to it (the app, the TUI, the core, a wrapper) is a finding.
      if (!contract && policy.linkedHost?.package === name && path.length > 1) {
        const parent = packages.get(path.at(-2)).name;
        if (path.length === 2 && policy.linkedHost.allowedParents.includes(parent)) continue;
        violations.push({ scope: 'hosts', package: name, path: path.map(id => packages.get(id).name) });
        continue;
      }
      if (path.length > 1) {
        const sourceMismatch = !contract && policy.contractManifests?.[name] &&
          packages.get(id).manifest_path !== policy.contractManifests[name];
        const forbidden = contract ? !CONTRACT_CLOSURE.has(name) || !contractSourceAllowed(name, packages.get(id).source) :
          isImplementation(name, policy) || sourceMismatch;
        if (forbidden) {
          const scope = contract ?? 'hosts';
          const parent = path.length > 1 ? packages.get(path.at(-2)).name : null;
          const exemption = policy.exceptions.find(item =>
            item.scope === scope && item.package === name &&
            (!item.parent || item.parent === parent));
          const finding = { scope, package: name, path: path.map(id => packages.get(id).name) };
          const findings = exemption ? exceptions : violations;
          const reportKey = `${exemption ? 'exception' : 'violation'}:${scope}:${id}`;
          if (!reportedEdges.has(reportKey)) {
            reportedEdges.add(reportKey);
            findings.push(finding);
          }
          // Contract exceptions pin the first unsafe dependency. Its implementation
          // closure is deliberately NOT approved vocabulary and must disappear
          // with this edge. Hosts still traverse it to catch indirect implementations.
          if (contract) continue;
        }
      }
      // Exceptions apply to incoming edges, not to package identities. A
      // second path can have a different parent and must be checked even when
      // this package's descendants have already been traversed.
      if (seen.has(id)) continue;
      seen.add(id);
      for (const next of edges(id)) {
        if (!contract && packages.get(next).name === policy.linkedHost?.package) {
          const parent = packages.get(id).name;
          if (path.length === 1 && policy.linkedHost.allowedParents.includes(parent)) continue;
          violations.push({
            scope: 'hosts', package: policy.linkedHost.package,
            path: [...path, next].map(item => packages.get(item).name),
          });
          continue;
        }
        queue.push([...path, next]);
      }
    }
  }
  return { violations, exceptions };
}

/**
 * Roots that reach the linked-modules bridge through normal or build edges.
 * Used on a metadata resolve WITHOUT `--all-features`: neither the CLI built
 * without `linked-modules`, nor the TUI, nor the core may reach it. Returns
 * violation findings shaped like {@link auditGraph}'s.
 */
export function bridgeReach(metadata, roots, policy) {
  const target = policy.linkedHost?.package;
  if (!target) return [];
  const { packages, edges } = graph(metadata);
  const found = [];
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
      if (packages.get(id).name === target) {
        found.push({ scope: 'hosts', package: target, path: path.map(item => packages.get(item).name) });
        continue;
      }
      for (const next of edges(id)) queue.push([...path, next]);
    }
  }
  return found;
}

/** Check bridge edges while accounting only for implementation edges in its closure. */
export function linkedHostAudit(metadata, root, policy) {
  const target = policy.linkedHost?.package;
  if (!target) return { violations: [], exceptions: [] };
  const { packages, edges } = graph(metadata);
  const candidates = [...packages.values()].filter(pkg => pkg.name === root);
  if (candidates.length !== 1) throw new Error(`expected exactly one root package ${root}`);
  const rootId = candidates[0].id;
  const packageKey = pkg => JSON.stringify([pkg.name, pkg.version, pkg.source, pkg.manifest_path]);
  const edgeKey = (parent, child) => `${packageKey(packages.get(parent))} -> ${packageKey(packages.get(child))}`;
  const bridgeRoots = edges(rootId).filter(id => packages.get(id).name === target);
  if (bridgeRoots.length === 0) throw new Error(`linked-modules graph for ${root} does not contain ${target}`);

  // The bridge subtree is an intentional local implementation link. Its
  // dependencies may also enable features on packages already in the host
  // graph, so allow only implementation edges proven to occur in that closure.
  const bridgeEdges = new Set();
  const bridgeQueue = [...bridgeRoots];
  const bridgeSeen = new Set();
  while (bridgeQueue.length) {
    const parent = bridgeQueue.pop();
    if (bridgeSeen.has(parent)) continue;
    bridgeSeen.add(parent);
    for (const child of edges(parent)) {
      bridgeEdges.add(edgeKey(parent, child));
      bridgeQueue.push(child);
    }
  }

  const queue = [[rootId]];
  const seen = new Set();
  const violations = [];
  const exceptions = [];
  const recorded = new Set();
  for (let at = 0; at < queue.length; at++) {
    const path = queue[at];
    const id = path.at(-1);
    if (seen.has(id)) continue;
    seen.add(id);
    for (const next of edges(id)) {
      if (packages.get(next).name === target) {
        if (id !== rootId || !policy.linkedHost.allowedParents.includes(root)) {
          violations.push({ scope: 'hosts', package: target, path: [...path, next].map(item => packages.get(item).name) });
        }
        continue;
      }
      const child = packages.get(next);
      const sourceMismatch = policy.contractManifests?.[child.name] &&
        child.manifest_path !== policy.contractManifests[child.name];
      const forbiddenImplementation = isImplementation(child.name, policy) && !bridgeEdges.has(edgeKey(id, next));
      if (sourceMismatch || forbiddenImplementation) {
        const key = `${child.name}:${edgeKey(id, next)}`;
        if (!recorded.has(key)) {
          recorded.add(key);
          const finding = { scope: 'hosts', package: child.name, path: [...path, next].map(item => packages.get(item).name) };
          const exemption = policy.exceptions.find(item => item.scope === 'hosts' && item.package === child.name);
          (exemption ? exceptions : violations).push(finding);
        }
      }
      queue.push([...path, next]);
    }
  }
  return { violations, exceptions };
}

export function validatePolicy(policy) {
  for (const key of ['implementationPackages', 'implementationPrefixes', 'hostAdapterPackages', 'contracts', 'pendingContracts', 'exceptions']) {
    if (!Array.isArray(policy[key])) throw new Error(`policy.${key} must be an array`);
  }
  if (!policy.contracts.length || !policy.implementationPackages.length || !policy.implementationPrefixes.length) {
    throw new Error('boundary inventory may not be empty');
  }
  if (policy.linkedHost !== undefined) {
    const { package: pkg, allowedParents } = policy.linkedHost;
    if (!pkg || !Array.isArray(allowedParents) || allowedParents.length === 0 || !allowedParents.every(Boolean)) {
      throw new Error('policy.linkedHost needs a package and a non-empty allowedParents list');
    }
    if (isImplementation(pkg, policy)) throw new Error('policy.linkedHost.package may not itself be an implementation package');
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
    if (!item.reason?.trim() || !item.package || !item.scope ||
        (item.parent !== undefined && !item.parent) || seen.has(key)) {
      throw new Error(`invalid or duplicate exception ${key}`);
    }
    if (item.scope === 'hosts' ? !isImplementation(item.package, policy) :
      !names.has(item.scope) || CONTRACT_CLOSURE.has(item.package)) {
      throw new Error(`exception ${key} does not name a forbidden dependency`);
    }
    seen.add(key);
  }
}

function cargoText(root, command, args, execute = execFileSync) {
  // All generated manifests and logs stay under the checkout's target/.
  const stdout = execute('bash', [join(root, 'scripts/ci-cancel-aware.sh'),
    'cargo', command, ...args], {
    cwd: root, encoding: 'utf8', maxBuffer: 128 * 1024 * 1024, stdio: ['ignore', 'pipe', 'inherit'],
  });
  return stdout;
}

function metadata(root, args, execute = execFileSync) {
  return JSON.parse(cargoText(root, 'metadata', ['--format-version', '1', ...args], execute));
}

function parseTreePackage(display) {
  const match = /^([^\s]+)\s+v([^\s]+)(.*)$/.exec(display);
  if (!match) throw new Error(`unrecognized Cargo tree package entry: ${display}`);
  const [, name, version, suffix] = match;
  const qualifiers = [...suffix.matchAll(/\(([^()]*)\)/g)].map(item => item[1]);
  const pathQualifier = qualifiers.find(item => isAbsolute(item) || item.startsWith('./') || item.startsWith('../'));
  const sourceQualifier = qualifiers.find(item => item !== pathQualifier && item !== 'proc-macro');
  let source = CONTRACT_REGISTRY;
  let manifestPath;
  if (pathQualifier) {
    source = null;
    manifestPath = join(resolve(pathQualifier), 'Cargo.toml');
  } else if (sourceQualifier) {
    source = sourceQualifier.startsWith('git+') || sourceQualifier.startsWith('registry+')
      ? sourceQualifier
      : `git+${sourceQualifier}`;
  }
  return { id: display, name, version, source, manifest_path: manifestPath };
}

/** Convert Cargo's package-scoped normal/build tree into the audit graph. */
export function treeGraph(output) {
  if (typeof output !== 'string' || !output.trim()) {
    throw new Error('Cargo dependency tree must be a non-empty string');
  }
  const packages = new Map();
  const nodes = new Map();
  const byDisplay = new Map();
  const stack = [];
  let root;
  for (const [index, line] of output.trimEnd().split('\n').entries()) {
    const match = /^(\d+)(.+)$/.exec(line);
    if (!match) throw new Error(`unrecognized Cargo tree line ${index + 1}`);
    const depth = Number(match[1]);
    const raw = match[2];
    const deduplicated = raw.endsWith(' (*)');
    const display = deduplicated ? raw.slice(0, -4) : raw;
    if (depth > stack.length) throw new Error(`Cargo tree skipped a depth at line ${index + 1}`);
    const parent = depth === 0 ? null : stack[depth - 1];
    if (depth > 0 && !parent) throw new Error(`Cargo tree has no parent at line ${index + 1}`);
    if (depth === 0) {
      if (root !== undefined) throw new Error('Cargo tree contains more than one root');
      root = display;
    }
    stack.length = depth;

    let id = byDisplay.get(display);
    if (deduplicated) {
      if (!id) throw new Error(`Cargo tree references an unseen deduplicated package: ${display}`);
    } else {
      if (!id) {
        const pkg = parseTreePackage(display);
        id = `tree:${index}:${display}`;
        packages.set(id, { ...pkg, id });
        nodes.set(id, { id, deps: [] });
        byDisplay.set(display, id);
      }
    }
    if (parent) {
      nodes.get(parent).deps.push({
        pkg: id,
        dep_kinds: [{ kind: null, target: null }],
      });
    }
    stack[depth] = id;
  }
  return { packages: [...packages.values()], resolve: { nodes: [...nodes.values()] }, root };
}

function hostTree(root, manifest, packageName, features, execute) {
  const args = [
    '--manifest-path', join(root, manifest),
    '--locked',
    '-p', packageName,
    '-e', 'normal,build',
    '--prefix', 'depth',
    '--format', '{p}',
  ];
  if (features === 'all') args.push('--all-features');
  else if (features === 'default') { /* Cargo's default feature set. */ }
  else if (Array.isArray(features)) {
    args.push('--features', features.join(','));
  } else throw new Error(`unsupported feature selection for ${packageName}`);
  const tree = treeGraph(cargoText(root, 'tree', args, execute));
  if (parseTreePackage(tree.root).name !== packageName) {
    throw new Error(`Cargo tree for ${packageName} was rooted at ${tree.root}`);
  }
  return tree;
}

function cliFeatures(root, execute) {
  const manifest = resolve(root, 'Cargo.toml');
  const listing = metadata(root, ['--manifest-path', manifest, '--no-deps'], execute);
  const pkg = listing.packages.find(item => item.name === 'openhuman-cli');
  if (!pkg || !pkg.features || !Object.hasOwn(pkg.features, 'linked-modules')) {
    throw new Error('openhuman-cli must declare its isolated linked-modules feature');
  }
  return Object.keys(pkg.features).filter(feature => feature !== 'default' && feature !== 'linked-modules');
}

function linkedCliTree(root, execute) {
  const tree = treeGraph(cargoText(root, 'tree', [
    '--manifest-path', join(root, 'Cargo.toml'), '--locked', '-p', 'openhuman-cli',
    '-e', 'normal,build', '--prefix', 'depth', '--format', '{p}',
    '--no-default-features', '--features', 'linked-modules',
  ], execute));
  if (parseTreePackage(tree.root).name !== 'openhuman-cli') {
    throw new Error(`linked-modules probe was not rooted at openhuman-cli: ${tree.root}`);
  }
  return tree;
}

/** Resolve only this contract's selected features, without module/host unification. */
function contractMetadata(root, contract, allFeatures, execute = execFileSync) {
  const manifest = resolve(root, contract.manifest);
  const listing = metadata(root, ['--manifest-path', manifest, '--no-deps'], execute);
  const pkg = listing.packages.find(pkg => resolve(pkg.manifest_path) === manifest);
  if (!pkg || pkg.name !== contract.name) throw new Error(`missing contract ${contract.name}`);
  const harness = join(root, 'target/module-boundaries', contract.name, allFeatures ? 'all' : 'default');
  mkdirSync(join(harness, 'src'), { recursive: true });
  const features = allFeatures ? Object.keys(pkg.features) : [];
  writeFileSync(join(harness, 'Cargo.toml'), `[workspace]\n[package]\nname = "boundary-probe"\nversion = "0.0.0"\nedition = "2024"\n[dependencies]\n${contract.name} = { path = ${JSON.stringify(dirname(manifest))}, features = ${JSON.stringify(features)} }\n`);
  writeFileSync(join(harness, 'src/lib.rs'), '');
  return metadata(root, ['--manifest-path', join(harness, 'Cargo.toml')], execute);
}

export function run(root, strict = false, execute = execFileSync) {
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
  for (const [manifest, packageName, features] of [
    ['Cargo.toml', 'openhuman', 'all'],
    ['Cargo.toml', 'openhuman-cli', cliFeatures(root, execute)],
    ['Cargo.toml', 'openhuman-tui', 'all'],
    ['crates/openhuman-app/Cargo.toml', 'openhuman-app', 'all'],
  ]) {
    const tree = hostTree(root, manifest, packageName, features, execute);
    results.push(auditGraph(tree, [packageName], policy));
  }
  // The linked-modules CLI feature is local-only, never shipped. Audit its
  // separately rooted graph for a direct CLI edge; do not let its feature
  // unification contaminate the CLI's shipping feature graph above.
  results.push(linkedHostAudit(linkedCliTree(root, execute), 'openhuman-cli', policy));
  // Default-feature graphs also guard against accidental bridge activation.
  for (const packageName of ['openhuman-cli', 'openhuman-tui', 'openhuman']) {
    results.push({
      violations: bridgeReach(
        hostTree(root, 'Cargo.toml', packageName, 'default', execute),
        [packageName],
        policy,
      ),
      exceptions: [],
    });
  }
  for (const contract of policy.contracts) {
    for (const all of [false, true]) {
      results.push(auditGraph(contractMetadata(root, contract, all, execute), [contract.name], policy, contract.name));
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
