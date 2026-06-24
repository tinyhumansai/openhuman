#!/usr/bin/env node
// @ts-check
/**
 * generate-architecture-docs.mjs
 * --------------------------------
 *
 * Minimal "docs generated from code" slice for issue #3892. Derives the
 * frontend **provider chain** from an explicit source marker in
 * `app/src/App.tsx` and renders it into a marked block in
 * `gitbooks/developing/architecture/frontend.md`, so the documented chain
 * cannot silently drift from the code (the exact drift called out in the
 * issue: the doc described `UserProvider` / `AIProvider` / `SkillProvider`
 * long after the code moved to `CoreStateProvider` / `ChatRuntimeProvider`).
 *
 * Modes:
 *   node scripts/generate-architecture-docs.mjs            # write (refresh docs)
 *   node scripts/generate-architecture-docs.mjs --check    # CI: fail on drift
 *
 * Source of truth: the `@generated-source:provider-chain` block in App.tsx.
 * This is an intentionally tiny, hand-auditable slice. Generating controller /
 * tool / skill reference tables from the Rust registry is the follow-up (see
 * the PR body); the splice + check harness here is built to extend to more
 * blocks without changing the CI wiring.
 */

import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(HERE, '..');

export const APP_TSX = resolve(REPO_ROOT, 'app/src/App.tsx');
export const FRONTEND_DOC = resolve(REPO_ROOT, 'gitbooks/developing/architecture/frontend.md');

/** Stable tokens used to locate the generated block in the doc. */
const BLOCK_BEGIN_TOKEN = 'BEGIN GENERATED: provider-chain';
const BLOCK_END_TOKEN = 'END GENERATED: provider-chain';

/**
 * Parse the ordered provider chain out of the `@generated-source:provider-chain`
 * marker block in App.tsx source text.
 *
 * @param {string} appSource - contents of App.tsx
 * @returns {{ order: number, name: string, role: string }[]}
 * @throws if the marker block is missing, empty, or rows are malformed / non-contiguous
 */
export function parseProviderChain(appSource) {
  const begin = appSource.indexOf('@generated-source:provider-chain');
  const end = appSource.indexOf('@end-source:provider-chain');
  if (begin === -1 || end === -1 || end < begin) {
    throw new Error(
      'provider-chain source marker not found in App.tsx; expected ' +
        '`@generated-source:provider-chain` … `@end-source:provider-chain`'
    );
  }
  const region = appSource.slice(begin, end);
  const rowRe = /^\s*\*?\s*(\d+)\.\s+(.+?)\s+—\s+(.+?)\s*$/;
  const providers = [];
  for (const line of region.split('\n')) {
    const m = rowRe.exec(line);
    if (!m) continue;
    providers.push({ order: Number(m[1]), name: m[2].trim(), role: m[3].trim() });
  }
  if (providers.length === 0) {
    throw new Error('provider-chain source marker contained no `N. Component — role` rows');
  }
  // Guard against drift in the marker itself: orders must be 1..N, in order,
  // names non-empty, and roles free of the `|` that would break the table.
  providers.forEach((p, i) => {
    if (p.order !== i + 1) {
      throw new Error(
        `provider-chain rows must be numbered 1..N in order; row ${i + 1} is numbered ${p.order}`
      );
    }
    if (!p.name) throw new Error(`provider-chain row ${p.order} is missing a component name`);
    if (!p.role) throw new Error(`provider-chain row ${p.order} is missing a role`);
    if (p.name.includes('|') || p.role.includes('|')) {
      throw new Error(`provider-chain row ${p.order} must not contain a "|" character`);
    }
  });
  return providers;
}

/**
 * Render the markdown body that lives between the BEGIN/END markers.
 * Returned as an array of lines (no trailing join) so splicing stays
 * deterministic and line-oriented.
 *
 * @param {{ order: number, name: string, role: string }[]} providers
 * @returns {string[]}
 */
export function renderProviderChainBody(providers) {
  return [
    '',
    '_Generated from `app/src/App.tsx` by `scripts/generate-architecture-docs.mjs`. ' +
      'Do not edit by hand — run `pnpm docs:generate` to refresh._',
    '',
    '| # | Component | Role |',
    '| --- | --- | --- |',
    ...providers.map(p => `| ${p.order} | \`${p.name}\` | ${p.role} |`),
    '',
  ];
}

/**
 * Splice freshly rendered body lines into `docSource` between the BEGIN/END
 * marker lines (markers preserved). Returns the updated document text.
 *
 * @param {string} docSource
 * @param {string[]} bodyLines
 * @returns {string}
 * @throws if the markers are missing or out of order
 */
export function spliceGeneratedBlock(docSource, bodyLines) {
  const lines = docSource.split('\n');
  const beginIdx = lines.findIndex(l => l.includes(BLOCK_BEGIN_TOKEN));
  const endIdx = lines.findIndex(l => l.includes(BLOCK_END_TOKEN));
  if (beginIdx === -1 || endIdx === -1) {
    throw new Error(
      `generated-block markers not found in ${FRONTEND_DOC}; expected HTML comments ` +
        `containing "${BLOCK_BEGIN_TOKEN}" and "${BLOCK_END_TOKEN}"`
    );
  }
  if (endIdx <= beginIdx) {
    throw new Error('generated-block END marker must come after the BEGIN marker');
  }
  const next = [...lines.slice(0, beginIdx + 1), ...bodyLines, ...lines.slice(endIdx)];
  return next.join('\n');
}

/**
 * Compute the desired doc content from the current sources.
 * @returns {{ updated: string, current: string }}
 */
export function computeFrontendDoc() {
  const providers = parseProviderChain(readFileSync(APP_TSX, 'utf8'));
  const current = readFileSync(FRONTEND_DOC, 'utf8');
  const updated = spliceGeneratedBlock(current, renderProviderChainBody(providers));
  return { updated, current };
}

function main() {
  const check = process.argv.includes('--check');
  const { updated, current } = computeFrontendDoc();

  if (check) {
    if (updated !== current) {
      console.error(
        '✗ Generated architecture docs are stale.\n' +
          `  ${FRONTEND_DOC} no longer matches its code source (app/src/App.tsx).\n` +
          '  Run `pnpm docs:generate` and commit the result.'
      );
      process.exit(1);
    }
    console.log('✓ Generated architecture docs are up to date.');
    return;
  }

  if (updated !== current) {
    writeFileSync(FRONTEND_DOC, updated);
    console.log(`✓ Regenerated provider-chain block in ${FRONTEND_DOC}`);
  } else {
    console.log('✓ Generated architecture docs already up to date — nothing to write.');
  }
}

// Only run when invoked directly, so the pure helpers above stay unit-testable.
if (resolve(fileURLToPath(import.meta.url)) === resolve(process.argv[1] ?? '')) {
  main();
}
