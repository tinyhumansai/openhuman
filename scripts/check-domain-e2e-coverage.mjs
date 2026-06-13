#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();

function usage() {
  return 'Usage: node scripts/check-domain-e2e-coverage.mjs';
}

for (const arg of process.argv.slice(2)) {
  if (arg === '--help' || arg === '-h') {
    console.log(usage());
    process.exit(0);
  }
  console.error(`check-domain-e2e-coverage: unknown argument: ${arg}`);
  console.error(usage());
  process.exit(2);
}

const rawThreshold = process.env.DOMAIN_E2E_COVERAGE_THRESHOLD ?? '90';
const THRESHOLD = Number(rawThreshold);
if (!Number.isFinite(THRESHOLD) || THRESHOLD < 0 || THRESHOLD > 100) {
  // A non-numeric value would make THRESHOLD NaN, turning every `percent <
  // THRESHOLD` comparison false and silently disabling the gate. Fail loudly.
  console.error(
    `Invalid DOMAIN_E2E_COVERAGE_THRESHOLD="${rawThreshold}". Expected a number between 0 and 100.`,
  );
  process.exit(2);
}

const MODULES = [
  { label: 'config', namespaces: ['config'] },
  { label: 'credentials', namespaces: ['auth'] },
  { label: 'app_state', namespaces: ['app_state'] },
  { label: 'connectivity', namespaces: ['connectivity'] },
  { label: 'inference', namespaces: ['inference'] },
  { label: 'agent', namespaces: ['agent'] },
  { label: 'tools', namespaces: ['tools'] },
  { label: 'tool_registry', namespaces: ['tool_registry'] },
  { label: 'approval', namespaces: ['approval'] },
  { label: 'memory', namespaces: ['memory'] },
  { label: 'memory_tree', namespaces: ['memory_tree'] },
  { label: 'memory_sync', namespaces: ['memory_sync'] },
  { label: 'memory_sources', namespaces: ['memory_sources'] },
  { label: 'embeddings', namespaces: ['embeddings'] },
  { label: 'channels', namespaces: ['channels'] },
  { label: 'composio', namespaces: ['composio'] },
  { label: 'threads', namespaces: ['threads'] },
];

const TARGET_NAMESPACES = new Set(MODULES.flatMap((module) => module.namespaces));

function walk(dir, predicate, out = []) {
  if (!fs.existsSync(dir)) return out;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      walk(full, predicate, out);
    } else if (predicate(full)) {
      out.push(full);
    }
  }
  return out;
}

function read(file) {
  return fs.readFileSync(file, 'utf8');
}

function collectInvokedMethods() {
  const methods = new Set();
  const testsDir = path.join(ROOT, 'tests');
  const files = walk(testsDir, (file) => file.endsWith('_e2e.rs'));

  for (const file of files) {
    const text = read(file);
    for (const match of text.matchAll(/"((?:openhuman)\.[A-Za-z0-9_]+)"/g)) {
      methods.add(match[1]);
    }
  }

  return methods;
}

function collectSchemaMethods() {
  const methodsByNamespace = new Map([...TARGET_NAMESPACES].map((namespace) => [namespace, new Set()]));
  const files = walk(path.join(ROOT, 'src', 'openhuman'), (file) => {
    const normalized = file.split(path.sep).join('/');
    return file.endsWith('.rs') && /(^|\/)schemas?(\.rs|\/)/.test(normalized);
  });

  for (const file of files) {
    const text = read(file);
    const constNamespace = text.match(/const\s+NAMESPACE:\s*&str\s*=\s*"([a-z_]+)"/)?.[1];
    for (const match of text.matchAll(/ControllerSchema\s*\{([\s\S]*?)\n\s*\}/g)) {
      const block = match[1];
      const namespaceToken = block.match(/namespace:\s*(?:NAMESPACE|"([a-z_]+)")/);
      const functionName = block.match(/function:\s*"([A-Za-z0-9_]+)"/)?.[1];
      const namespace = namespaceToken?.[1] ?? (namespaceToken ? constNamespace : undefined);
      if (!namespace || !functionName || functionName === 'unknown') continue;
      if (!TARGET_NAMESPACES.has(namespace)) continue;
      methodsByNamespace.get(namespace).add(`openhuman.${namespace}_${functionName}`);
    }
  }

  return methodsByNamespace;
}

const invoked = collectInvokedMethods();
const schemas = collectSchemaMethods();
let failed = false;

console.log(`Domain Rust E2E controller coverage threshold: ${THRESHOLD}%`);
console.log('');
console.log('| Module | Namespace(s) | Covered | Percent | Missing |');
console.log('| --- | --- | ---: | ---: | --- |');

for (const module of MODULES) {
  const expected = new Set();
  const covered = new Set();
  for (const namespace of module.namespaces) {
    for (const method of schemas.get(namespace) ?? []) expected.add(method);
  }
  for (const method of expected) {
    if (invoked.has(method)) covered.add(method);
  }

  const missing = [...expected].filter((method) => !covered.has(method)).sort();
  const percent = expected.size === 0 ? 100 : (covered.size / expected.size) * 100;
  const missingText = missing.length === 0 ? '-' : missing.join('<br>');

  console.log(
    `| ${module.label} | ${module.namespaces.join(', ')} | ${covered.size}/${expected.size} | ${percent.toFixed(1)}% | ${missingText} |`,
  );

  if (expected.size > 0 && percent < THRESHOLD) failed = true;
}

if (failed) {
  console.error(`\nDomain Rust E2E controller coverage is below ${THRESHOLD}% for one or more modules.`);
  process.exit(1);
}

console.log(`\nAll named modules meet the ${THRESHOLD}% Rust E2E controller coverage threshold.`);
