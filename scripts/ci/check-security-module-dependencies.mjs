// Enforce the security module's bus-only boundary before migration begins.
// Parse TOML rather than scanning lines: Cargo supports package aliases,
// per-dependency tables, quoted keys and target-specific dependencies.
import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { parse } from 'smol-toml';

export const HOST_MANIFESTS = [
  'Cargo.toml',
  ...['core', 'embed', 'tinyhumans', 'rpc', 'app', 'cli', 'tui'].map(
    (name) => `crates/openhuman-${name}/Cargo.toml`
  ),
];
const DEPENDENCY_TABLES = new Set(['dependencies', 'build-dependencies', 'dev-dependencies']);

export function checkManifest(source, filename = 'Cargo.toml') {
  const manifest = parse(source);
  const violations = [];
  function dependencies(table, location) {
    for (const [alias, dependency] of Object.entries(table ?? {})) {
      const pkg = typeof dependency === 'object' ? dependency.package ?? alias : alias;
      if (typeof pkg !== 'string') throw new Error(`${filename}: invalid package in ${location}.${alias}`);
      if ((pkg === 'tinysecurity' || pkg.startsWith('tinysecurity-')) && pkg !== 'tinysecurity-bus') {
        violations.push(`${filename}: ${location}.${alias} links ${pkg}; only tinysecurity-bus is allowed`);
      }
    }
  }
  for (const name of DEPENDENCY_TABLES) {
    dependencies(manifest[name], name);
    dependencies(manifest.workspace?.[name], `workspace.${name}`);
    for (const [target, table] of Object.entries(manifest.target ?? {})) {
      dependencies(table[name], `target.${target}.${name}`);
    }
  }
  for (const [registry, table] of Object.entries(manifest.patch ?? {})) {
    dependencies(table, `patch.${registry}`);
  }
  // Cargo's legacy replacement table also contains package references.
  for (const [key, dependency] of Object.entries(manifest.replace ?? {})) {
    const alias = key.split(':')[0];
    dependencies({ [alias]: dependency }, 'replace');
  }
  return violations;
}

export function checkRepository(root) {
  const manifests = new Set(HOST_MANIFESTS);
  function walk(dir) {
    for (const entry of readdirSync(join(root, dir), { withFileTypes: true })) {
      if (['target', 'vendor', 'worktrees', '.git'].includes(entry.name)) continue;
      const path = join(dir, entry.name);
      if (entry.isDirectory()) walk(path);
      else if (entry.name === 'Cargo.toml') manifests.add(path);
    }
  }
  // Only the host's crates tree is included; vendored repositories and other
  // workflow checkouts independently enforce their own dependency boundaries.
  walk('crates');
  return [...manifests].flatMap((path) => checkManifest(readFileSync(join(root, path), 'utf8'), path));
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const root = resolve(process.argv[2] ?? join(dirname(fileURLToPath(import.meta.url)), '../..'));
  try {
    const violations = checkRepository(root);
    if (violations.length) {
      console.error(violations.join('\n'));
      process.exitCode = 1;
    } else console.log('Security module dependency boundary: PASS');
  } catch (error) {
    console.error(`Security module dependency check refused unreadable or invalid input: ${error.message}`);
    process.exitCode = 2;
  }
}
