import { describe, expect, it } from 'vitest';

import { NullEmbeddingProvider } from '../embeddings';

describe('NullEmbeddingProvider', () => {
  it('should have correct id and model', () => {
    const provider = new NullEmbeddingProvider();
    expect(provider.id).toBe('null');
    expect(provider.model).toBe('none');
    expect(provider.dimensions).toBe(0);
  });

  it('should return empty array for embedQuery', async () => {
    const provider = new NullEmbeddingProvider();
    const result = await provider.embedQuery('hello world');
    expect(result).toEqual([]);
  });

  it('should return empty arrays for embedBatch', async () => {
    const provider = new NullEmbeddingProvider();
    const result = await provider.embedBatch(['hello', 'world', 'test']);
    expect(result).toHaveLength(3);
    expect(result[0]).toEqual([]);
    expect(result[1]).toEqual([]);
    expect(result[2]).toEqual([]);
  });

  it('should handle empty batch', async () => {
    const provider = new NullEmbeddingProvider();
    const result = await provider.embedBatch([]);
    expect(result).toHaveLength(0);
  });
});
