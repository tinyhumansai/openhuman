#!/usr/bin/env node
import { createHash } from 'node:crypto';
import { readdirSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const appRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const contractPath = path.join(appRoot, 'scripts/e2e-web-selection-contract.json');
const allowedDiagnosticPaths = new Set(['test/playwright/specs/aui-tool-result-elements.spec.ts']);

function sortedUnique(paths, label) {
  const sorted = [...paths].sort();
  if (new Set(sorted).size !== sorted.length) {
    throw new Error(`${label} contains duplicate spec paths`);
  }
  return sorted;
}

export function classifySpecFiles(discoveredPaths, contract) {
  const discovered = sortedUnique(discoveredPaths, 'discovery');
  if (discovered.length === 0) throw new Error('Playwright spec discovery is empty');
  const discoveredSet = new Set(discovered);
  const diagnosticEntries = contract.diagnostics;
  if (!Array.isArray(diagnosticEntries) || diagnosticEntries.length === 0) {
    throw new Error('diagnostic inventory must be non-empty');
  }

  const diagnostic = sortedUnique(
    diagnosticEntries.map(entry => entry.path),
    'diagnostic inventory'
  );
  const unknownDiagnosticPolicy = diagnostic.filter(file => !allowedDiagnosticPaths.has(file));
  if (unknownDiagnosticPolicy.length || diagnostic.length !== allowedDiagnosticPaths.size) {
    throw new Error(
      `diagnostic inventory differs from the reviewed file allowlist: ${diagnostic.join(', ')}`
    );
  }
  const unknownDiagnostics = diagnostic.filter(file => !discoveredSet.has(file));
  if (unknownDiagnostics.length) {
    throw new Error(`stale or unknown diagnostic spec: ${unknownDiagnostics.join(', ')}`);
  }

  const owners = sortedUnique(contract.protectedOwnerFiles, 'protected owner inventory');
  const unknownOwners = owners.filter(file => !discoveredSet.has(file));
  if (unknownOwners.length) {
    throw new Error(`stale or unknown protected owner spec: ${unknownOwners.join(', ')}`);
  }
  const demotedOwners = owners.filter(file => diagnostic.includes(file));
  if (demotedOwners.length) {
    throw new Error(`protected browser owners cannot be diagnostic: ${demotedOwners.join(', ')}`);
  }

  const overlap = diagnostic.filter(file => owners.includes(file));
  if (overlap.length)
    throw new Error(`diagnostic and required owner overlap: ${overlap.join(', ')}`);
  const required = discovered.filter(file => !diagnostic.includes(file));
  if (required.length === 0 || diagnostic.length === 0) {
    throw new Error('required and diagnostic selections must both be non-empty');
  }
  if (new Set([...required, ...diagnostic]).size !== discovered.length) {
    throw new Error('required and diagnostic selections are not disjoint and exhaustive');
  }

  for (const entry of diagnosticEntries) {
    if (
      !/^https:\/\/github\.com\/[^/]+\/[^/]+\/issues\/\d+#issuecomment-\d+$/.test(
        entry.issue ?? ''
      ) ||
      !/^\d+$/.test(entry.baselineRun ?? '') ||
      !/^\d+$/.test(entry.afterRun ?? '') ||
      !/^[a-f0-9]{40}$/.test(entry.baselineHead ?? '') ||
      !/^[a-f0-9]{40}$/.test(entry.afterHead ?? '') ||
      !entry.caseLabel ||
      !entry.reason ||
      !/^[a-f0-9]{64}$/.test(entry.sha256 ?? '')
    ) {
      throw new Error(`diagnostic entry lacks linked issue and immutable evidence: ${entry.path}`);
    }
  }

  return { all: discovered, required, diagnostic };
}

export function verifyDiagnosticBytes(contract, readFile = readFileSync) {
  for (const entry of contract.diagnostics) {
    const absolutePath = path.join(appRoot, entry.path);
    const digest = createHash('sha256').update(readFile(absolutePath)).digest('hex');
    if (digest !== entry.sha256) {
      throw new Error(
        `diagnostic source changed for ${entry.path}; review and resolve its exact issue before scheduling`
      );
    }
  }
}

export function discoverSpecFiles() {
  const specsRoot = path.join(appRoot, 'test/playwright/specs');
  const files = [];
  const visit = directory => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const absolutePath = path.join(directory, entry.name);
      if (entry.isDirectory()) visit(absolutePath);
      else if (entry.isFile() && entry.name.endsWith('.spec.ts')) {
        files.push(path.relative(appRoot, absolutePath).split(path.sep).join('/'));
      }
    }
  };

  visit(specsRoot);
  return files.sort();
}

export function resolveSelection(selection, { contract, discovered = discoverSpecFiles() }) {
  if (!['all', 'required', 'diagnostic'].includes(selection)) {
    throw new Error(`unknown Playwright selection: ${selection}`);
  }
  const sets = classifySpecFiles(discovered, contract);
  verifyDiagnosticBytes(contract);
  const files = sets[selection];
  if (!files.length) throw new Error(`${selection} Playwright selection is empty`);
  return { selection, files, ...sets };
}

function main(args) {
  let selection = 'all';
  let asJson = false;
  for (const arg of args) {
    if (arg.startsWith('--selection=')) selection = arg.slice('--selection='.length);
    else if (arg === '--json') asJson = true;
    else throw new Error(`unknown argument: ${arg}`);
  }
  const contract = JSON.parse(readFileSync(contractPath, 'utf8'));
  const result = resolveSelection(selection, { contract });
  process.stdout.write(asJson ? `${JSON.stringify(result)}\n` : `${result.files.join('\n')}\n`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
    process.exitCode = 2;
  }
}
