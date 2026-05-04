import { describe, expect, it } from 'vitest';

import { __MOCK_CHUNKS__, MEMORY_TREE_USE_MOCK, memoryTreeApi } from './memoryTreeApi';

describe('memoryTreeApi (mocks)', () => {
  it('starts in mock mode by default', () => {
    expect(MEMORY_TREE_USE_MOCK).toBe(true);
  });

  it('seeds at least 30 chunks across multiple sources', () => {
    expect(__MOCK_CHUNKS__.length).toBeGreaterThanOrEqual(20);
    const sourceIds = new Set(__MOCK_CHUNKS__.map(c => c.source_id));
    expect(sourceIds.size).toBeGreaterThanOrEqual(8);
  });

  it('listChunks returns chunks sorted newest-first', async () => {
    const result = await memoryTreeApi.listChunks({ limit: 10 });
    expect(result.chunks.length).toBeLessThanOrEqual(10);
    for (let i = 1; i < result.chunks.length; i++) {
      expect(result.chunks[i - 1].timestamp_ms).toBeGreaterThanOrEqual(
        result.chunks[i].timestamp_ms
      );
    }
  });

  it('listChunks honours the source_ids filter', async () => {
    const all = await memoryTreeApi.listChunks({ limit: 500 });
    const onlySource = all.chunks[0].source_id;
    const filtered = await memoryTreeApi.listChunks({ source_ids: [onlySource] });
    expect(filtered.chunks.length).toBeGreaterThan(0);
    expect(filtered.chunks.every(c => c.source_id === onlySource)).toBe(true);
  });

  it('listSources groups chunks by source and returns derived display names', async () => {
    const sources = await memoryTreeApi.listSources();
    expect(sources.length).toBeGreaterThan(0);
    const steve = sources.find(s => s.display_name === 'Steven Enamakel');
    expect(steve).toBeDefined();
    expect(steve?.chunk_count).toBeGreaterThan(0);
    expect(steve?.lifecycle_status).toMatch(/admitted|buffered|pending_extraction|dropped/);
  });

  it('topEntities filters by kind=person', async () => {
    const people = await memoryTreeApi.topEntities('person', 5);
    expect(people.length).toBeGreaterThan(0);
    expect(people.every(p => p.kind === 'person')).toBe(true);
    // Sorted by count desc
    for (let i = 1; i < people.length; i++) {
      expect(people[i - 1].count).toBeGreaterThanOrEqual(people[i].count);
    }
  });

  it("entityIndexFor returns refs derived from a chunk's tags", async () => {
    const sample = __MOCK_CHUNKS__[0];
    const refs = await memoryTreeApi.entityIndexFor(sample.id);
    expect(refs.length).toBe(sample.tags.length);
    for (const ref of refs) {
      expect(ref.surface.length).toBeGreaterThan(0);
      expect(ref.count).toBeGreaterThanOrEqual(1);
    }
  });

  it('chunkScore returns a breakdown with three weighted signals summing within tolerance', async () => {
    const sample = __MOCK_CHUNKS__[0];
    const breakdown = await memoryTreeApi.chunkScore(sample.id);
    expect(breakdown.signals).toHaveLength(3);
    const reconstructed = breakdown.signals.reduce((sum, sig) => sum + sig.weight * sig.value, 0);
    expect(Math.abs(reconstructed - breakdown.total)).toBeLessThan(0.05);
    expect(breakdown.threshold).toBeGreaterThan(0);
    expect(breakdown.threshold).toBeLessThanOrEqual(1);
  });

  it('search narrows by query string', async () => {
    const hits = await memoryTreeApi.search('PR #1175', 20);
    expect(hits.length).toBeGreaterThan(0);
    expect(hits.every(c => /1175/.test(c.content_preview ?? ''))).toBe(true);
  });

  it('recall returns chunks with descending mock scores', async () => {
    const result = await memoryTreeApi.recall('memory tree design', 5);
    expect(result.chunks.length).toBe(result.scores.length);
    for (let i = 1; i < result.scores.length; i++) {
      expect(result.scores[i - 1]).toBeGreaterThanOrEqual(result.scores[i]);
    }
  });

  it('deleteChunk resolves cleanly in mock mode', async () => {
    await expect(memoryTreeApi.deleteChunk('chunk-354fa083')).resolves.toBeUndefined();
  });
});
