#!/usr/bin/env node
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { parseMatrix, validateAgainstCatalog } from './lib/coverage-matrix-parser.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, '..');

let matrixMd;
let catalog;
try {
  matrixMd = await readFile(join(repoRoot, 'docs/TEST-COVERAGE-MATRIX.md'), 'utf8');
  catalog = JSON.parse(await readFile(join(repoRoot, 'scripts/feature-ids.json'), 'utf8'));
} catch (err) {
  console.error(`Failed to read inputs: ${err.message}`);
  process.exit(1);
}

const parsed = parseMatrix(matrixMd);
const validation = validateAgainstCatalog(parsed.rows, catalog.ids);

let failed = false;
if (parsed.errors.length) {
  console.error('Matrix parse errors:');
  for (const err of parsed.errors) console.error(`  - ${err}`);
  failed = true;
}
if (validation.missingFromMatrix.length) {
  console.error('Catalog IDs missing from matrix:');
  for (const id of validation.missingFromMatrix) console.error(`  - ${id}`);
  failed = true;
}
if (validation.duplicates.length) {
  console.error('Duplicate IDs in matrix:');
  for (const id of validation.duplicates) console.error(`  - ${id}`);
  failed = true;
}

console.log(
  `Matrix: ${parsed.rows.length} rows, ${catalog.ids.length} catalog IDs, ${parsed.errors.length} parse errors, ${validation.missingFromMatrix.length} missing, ${validation.duplicates.length} duplicates`,
);
process.exit(failed ? 1 : 0);
