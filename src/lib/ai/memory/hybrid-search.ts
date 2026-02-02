import { invoke } from '@tauri-apps/api/core';

import type { EmbeddingProvider } from '../providers/embeddings';
import { DEFAULT_MEMORY_CONFIG, type MemoryConfig, type SearchResult } from './types';

/** Raw search result from Rust FTS5 */
interface FtsSearchResult {
  chunk_id: string;
  path: string;
  source: string;
  text: string;
  score: number;
  start_line: number;
  end_line: number;
  updated_at?: number;
}

/**
 * Compute cosine similarity between two vectors.
 */
function cosineSimilarity(a: number[], b: number[]): number {
  if (a.length !== b.length) return 0;

  let dotProduct = 0;
  let normA = 0;
  let normB = 0;

  for (let i = 0; i < a.length; i++) {
    dotProduct += a[i] * b[i];
    normA += a[i] * a[i];
    normB += b[i] * b[i];
  }

  const denominator = Math.sqrt(normA) * Math.sqrt(normB);
  return denominator === 0 ? 0 : dotProduct / denominator;
}

/**
 * Decode embedding bytes (Float32Array stored as byte array) back to number[].
 */
function decodeEmbedding(bytes: number[]): number[] {
  const buffer = new ArrayBuffer(bytes.length);
  const view = new Uint8Array(buffer);
  for (let i = 0; i < bytes.length; i++) {
    view[i] = bytes[i];
  }
  return Array.from(new Float32Array(buffer));
}

/**
 * Perform hybrid search combining vector similarity and FTS5 keyword search.
 *
 * Algorithm:
 * 1. Get query embedding from provider
 * 2. Run FTS5 keyword search in SQLite
 * 3. Run vector similarity search against all stored embeddings
 * 4. Merge results with weighted scoring
 */
export async function hybridSearch(
  query: string,
  embeddingProvider: EmbeddingProvider | null,
  config: Partial<MemoryConfig> = {}
): Promise<SearchResult[]> {
  const { vectorWeight, textWeight, maxResults } = { ...DEFAULT_MEMORY_CONFIG, ...config };

  // Run FTS5 search
  const ftsResults = await invoke<FtsSearchResult[]>('ai_memory_fts_search', {
    query,
    limit: maxResults * 2,
  });

  // Normalize FTS scores to 0-1 range
  const maxFtsScore = Math.max(...ftsResults.map(r => r.score), 1);
  const ftsScoreMap = new Map<string, { result: FtsSearchResult; score: number }>();
  for (const r of ftsResults) {
    ftsScoreMap.set(r.chunk_id, { result: r, score: r.score / maxFtsScore });
  }

  // Vector search (if embedding provider available)
  const vectorScoreMap = new Map<string, number>();

  if (embeddingProvider) {
    try {
      const queryEmbedding = await embeddingProvider.embedQuery(query);
      const allEmbeddings = await invoke<[string, number[]][]>('ai_memory_get_all_embeddings');

      for (const [chunkId, embeddingBytes] of allEmbeddings) {
        const embedding = decodeEmbedding(embeddingBytes);
        const similarity = cosineSimilarity(queryEmbedding, embedding);
        vectorScoreMap.set(chunkId, similarity);
      }
    } catch {
      // Vector search failed — fall back to FTS only
    }
  }

  // Merge results with weighted scoring
  const allChunkIds = new Set([...ftsScoreMap.keys(), ...vectorScoreMap.keys()]);

  const mergedResults: SearchResult[] = [];

  for (const chunkId of allChunkIds) {
    const ftsEntry = ftsScoreMap.get(chunkId);
    const vectorScore = vectorScoreMap.get(chunkId) ?? 0;
    const ftsScore = ftsEntry?.score ?? 0;

    const combinedScore = vectorWeight * vectorScore + textWeight * ftsScore;

    // We need the text and metadata — get from FTS result if available
    if (ftsEntry) {
      mergedResults.push({
        chunkId,
        path: ftsEntry.result.path,
        source: ftsEntry.result.source as 'memory' | 'sessions',
        text: ftsEntry.result.text,
        score: combinedScore,
        startLine: ftsEntry.result.start_line,
        endLine: ftsEntry.result.end_line,
        updatedAt: ftsEntry.result.updated_at,
      });
    } else if (vectorScore > 0) {
      // Chunk found via vector search only — need to fetch details
      // This is handled by adding score placeholder, details filled by caller
      mergedResults.push({
        chunkId,
        path: '',
        source: 'memory',
        text: '',
        score: combinedScore,
        startLine: 0,
        endLine: 0,
      });
    }
  }

  // Sort by score descending and limit
  mergedResults.sort((a, b) => b.score - a.score);
  return mergedResults.slice(0, maxResults);
}
