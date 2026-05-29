import { beforeEach, describe, expect, it, vi } from 'vitest';

import { buildBeliefHistories } from '../../lib/memory/beliefLedger';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import {
  beliefLedgerApi,
  buildExplanationPrompt,
  correctionKey,
  explainConflict,
  loadRelations,
  saveCorrection,
} from './beliefLedgerApi';

const mockMemoryGraphQuery = vi.fn();
const mockMemoryDocIngest = vi.fn();
const mockAgentChat = vi.fn();

vi.mock('../../utils/tauriCommands/memory', () => ({
  memoryGraphQuery: (...args: unknown[]) => mockMemoryGraphQuery(...args),
  memoryDocIngest: (...args: unknown[]) => mockMemoryDocIngest(...args),
}));
vi.mock('../../utils/tauriCommands/localAi', () => ({
  openhumanAgentChat: (...args: unknown[]) => mockAgentChat(...args),
}));

const NOW = 1_700_000_000;
const DAY = 86_400;

function makeRel(overrides: Partial<GraphRelation> = {}): GraphRelation {
  return {
    namespace: 'work',
    subject: 'You',
    predicate: 'lives_in',
    object: 'Berlin',
    attrs: {},
    updatedAt: NOW,
    evidenceCount: 1,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
    ...overrides,
  };
}

const contestedHistory = buildBeliefHistories(
  [
    makeRel({
      predicate: 'lives_in',
      object: 'Berlin',
      updatedAt: NOW - 120 * DAY,
      evidenceCount: 3,
    }),
    makeRel({ predicate: 'lives_in', object: 'Lisbon', updatedAt: NOW, evidenceCount: 2 }),
  ],
  NOW
)[0];

describe('beliefLedgerApi.loadRelations', () => {
  beforeEach(() => {
    mockMemoryGraphQuery.mockReset();
  });

  it('calls memoryGraphQuery with the namespace and returns the relations', async () => {
    const rels = [makeRel()];
    mockMemoryGraphQuery.mockResolvedValueOnce(rels);
    const out = await loadRelations('work');
    expect(mockMemoryGraphQuery).toHaveBeenCalledWith('work');
    expect(out).toBe(rels);
  });

  it('passes undefined namespace through (all namespaces)', async () => {
    mockMemoryGraphQuery.mockResolvedValueOnce([]);
    await loadRelations();
    expect(mockMemoryGraphQuery).toHaveBeenCalledWith(undefined);
  });
});

describe('buildExplanationPrompt', () => {
  it('lists every observed value newest-first and tags the current belief', () => {
    const prompt = buildExplanationPrompt(contestedHistory);
    expect(prompt).toContain('lives_in');
    expect(prompt).toContain('"Lisbon"');
    expect(prompt).toContain('"Berlin"');
    expect(prompt).toContain('[current best belief]');
    // Newest (Lisbon) appears before older (Berlin).
    expect(prompt.indexOf('Lisbon')).toBeLessThan(prompt.indexOf('Berlin'));
    // Deterministic ISO date derived from the fixed timestamp.
    expect(prompt).toContain('evidence x');
  });
});

describe('beliefLedgerApi.explainConflict', () => {
  beforeEach(() => {
    mockAgentChat.mockReset();
  });

  it('sends the built prompt to openhumanAgentChat and returns .result', async () => {
    mockAgentChat.mockResolvedValueOnce({ result: 'They moved from Berlin to Lisbon.', logs: [] });
    const out = await explainConflict(contestedHistory);
    expect(mockAgentChat).toHaveBeenCalledTimes(1);
    expect(mockAgentChat).toHaveBeenCalledWith(buildExplanationPrompt(contestedHistory));
    expect(out).toBe('They moved from Berlin to Lisbon.');
  });

  it('propagates errors from the agent call', async () => {
    mockAgentChat.mockRejectedValueOnce(new Error('model unavailable'));
    await expect(explainConflict(contestedHistory)).rejects.toThrow('model unavailable');
  });
});

describe('correctionKey', () => {
  it('is a deterministic, normalized, charset-safe key', () => {
    expect(
      correctionKey({
        namespace: 'work',
        subject: 'You',
        predicate: 'lives_in',
        correctObject: 'Lisbon',
      })
    ).toBe('belief-correction:work:you:lives%20in');
    // Case/underscore variants of the SAME claim -> same key (re-correction overwrites).
    expect(
      correctionKey({
        namespace: 'work',
        subject: 'You',
        predicate: 'Lives In',
        correctObject: 'X',
      })
    ).toBe('belief-correction:work:you:lives%20in');
  });

  it('keeps DISTINCT non-Latin claims distinct (no empty-slug collision)', () => {
    const a = correctionKey({
      namespace: 'ns',
      subject: '北京',
      predicate: '位于',
      correctObject: '中国',
    });
    const b = correctionKey({
      namespace: 'ns',
      subject: '東京',
      predicate: '位于',
      correctObject: '日本',
    });
    expect(a).not.toBe(b);
  });
});

describe('beliefLedgerApi.saveCorrection', () => {
  beforeEach(() => {
    mockMemoryDocIngest.mockReset();
  });

  it('ingests a correction note with the right metadata + source/category tags', async () => {
    mockMemoryDocIngest.mockResolvedValueOnce(undefined);
    await saveCorrection({
      namespace: 'work',
      subject: 'You',
      predicate: 'lives_in',
      correctObject: 'Lisbon',
    });
    expect(mockMemoryDocIngest).toHaveBeenCalledTimes(1);
    const arg = mockMemoryDocIngest.mock.calls[0][0];
    expect(arg.namespace).toBe('work');
    expect(arg.key).toBe('belief-correction:work:you:lives%20in');
    expect(arg.source_type).toBe('llm_thought');
    expect(arg.category).toBe('correction');
    expect(arg.tags).toEqual(['belief-correction']);
    expect(arg.metadata).toEqual({ subject: 'You', predicate: 'lives_in', object: 'Lisbon' });
    expect(arg.content).toContain('Lisbon');
  });

  it('propagates errors from the ingest call', async () => {
    mockMemoryDocIngest.mockRejectedValueOnce(new Error('ingest failed'));
    await expect(
      saveCorrection({
        namespace: 'work',
        subject: 'You',
        predicate: 'lives_in',
        correctObject: 'Lisbon',
      })
    ).rejects.toThrow('ingest failed');
  });
});

describe('beliefLedgerApi object', () => {
  it('exposes the public surface', () => {
    expect(typeof beliefLedgerApi.loadRelations).toBe('function');
    expect(typeof beliefLedgerApi.explainConflict).toBe('function');
    expect(typeof beliefLedgerApi.saveCorrection).toBe('function');
  });
});
