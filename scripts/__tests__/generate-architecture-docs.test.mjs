// Tests for the architecture-docs generator (issue #3892).
// Run: node --test scripts/__tests__/generate-architecture-docs.test.mjs
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

import {
  APP_TSX,
  FRONTEND_DOC,
  computeFrontendDoc,
  parseProviderChain,
  renderProviderChainBody,
  spliceGeneratedBlock,
} from '../generate-architecture-docs.mjs';

const SAMPLE = `
  /*
   * @generated-source:provider-chain
   * 1. Sentry.ErrorBoundary — Crash boundary; renders ErrorFallbackScreen
   * 2. Provider — Redux store; enables useAppSelector / dispatch app-wide
   * @end-source:provider-chain
   */
`;

test('parseProviderChain extracts ordered rows from the marker block', () => {
  const providers = parseProviderChain(SAMPLE);
  assert.equal(providers.length, 2);
  assert.deepEqual(providers[0], {
    order: 1,
    name: 'Sentry.ErrorBoundary',
    role: 'Crash boundary; renders ErrorFallbackScreen',
  });
  assert.equal(providers[1].name, 'Provider');
});

test('parseProviderChain throws when the marker block is missing', () => {
  assert.throws(() => parseProviderChain('// no markers here'), /source marker not found/);
});

test('parseProviderChain throws when ordering is non-contiguous (drifted metadata)', () => {
  const bad = `@generated-source:provider-chain
   * 1. A — first
   * 3. C — third
   @end-source:provider-chain`;
  assert.throws(() => parseProviderChain(bad), /numbered 1\.\.N/);
});

test('parseProviderChain rejects a role containing a table-breaking pipe', () => {
  const bad = `@generated-source:provider-chain
   * 1. A — has a | pipe
   @end-source:provider-chain`;
  assert.throws(() => parseProviderChain(bad), /must not contain a "\|"/);
});

test('renderProviderChainBody is deterministic and table-shaped', () => {
  const body = renderProviderChainBody(parseProviderChain(SAMPLE));
  assert.equal(body[0], '');
  assert.equal(body[3], '| # | Component | Role |');
  assert.equal(body[4], '| --- | --- | --- |');
  assert.equal(body[5], '| 1 | `Sentry.ErrorBoundary` | Crash boundary; renders ErrorFallbackScreen |');
  assert.equal(body[body.length - 1], '');
});

test('spliceGeneratedBlock replaces only the content between markers', () => {
  const doc = [
    '# Title',
    '<!-- BEGIN GENERATED: provider-chain -->',
    'stale row',
    '<!-- END GENERATED: provider-chain -->',
    'after',
  ].join('\n');
  const out = spliceGeneratedBlock(doc, ['', 'fresh', '']);
  assert.equal(
    out,
    [
      '# Title',
      '<!-- BEGIN GENERATED: provider-chain -->',
      '',
      'fresh',
      '',
      '<!-- END GENERATED: provider-chain -->',
      'after',
    ].join('\n')
  );
});

test('spliceGeneratedBlock throws when markers are absent', () => {
  assert.throws(() => spliceGeneratedBlock('no markers', ['x']), /markers not found/);
});

test('committed frontend.md is in sync with App.tsx (the CI drift gate)', () => {
  const { updated, current } = computeFrontendDoc();
  assert.equal(updated, current, 'run `pnpm docs:generate` and commit the result');
});

test('App.tsx still reflects the post-CoreState/ChatRuntime provider chain', () => {
  // Guards against regressing the doc back to the pre-audit provider names.
  const names = parseProviderChain(readFileSync(APP_TSX, 'utf8')).map(p => p.name);
  assert.ok(names.includes('CoreStateProvider'));
  assert.ok(names.includes('ChatRuntimeProvider'));
  assert.ok(!names.includes('UserProvider'), 'UserProvider was removed pre-audit');
  assert.ok(!names.includes('AIProvider'), 'AIProvider was removed pre-audit');
  assert.ok(!names.includes('SkillProvider'), 'SkillProvider was removed pre-audit');
  // Sanity: the doc path the gate guards actually exists.
  assert.ok(FRONTEND_DOC.endsWith('gitbooks/developing/architecture/frontend.md'));
});
