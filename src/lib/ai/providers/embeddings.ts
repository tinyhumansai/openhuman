/**
 * Embedding provider interface and implementations.
 *
 * Embeddings are used for vector similarity search in the memory system.
 * The provider is abstracted to support different backends.
 */

/** Abstract embedding provider */
export interface EmbeddingProvider {
  /** Provider identifier */
  id: string;
  /** Model name used for embeddings */
  model: string;
  /** Embedding dimension count */
  dimensions: number;

  /** Embed a single query text */
  embedQuery(text: string): Promise<number[]>;
  /** Embed a batch of texts */
  embedBatch(texts: string[]): Promise<number[][]>;
}

/** Configuration for an embedding provider */
export interface EmbeddingProviderConfig {
  /** Provider identifier */
  id: string;
  /** API endpoint URL */
  endpoint?: string;
  /** API key */
  apiKey?: string;
  /** Model name */
  model?: string;
}

/**
 * No-op embedding provider for when no external API is configured.
 * Returns zero vectors, effectively disabling vector search.
 */
export class NullEmbeddingProvider implements EmbeddingProvider {
  id = 'null';
  model = 'none';
  dimensions = 0;

  async embedQuery(_text: string): Promise<number[]> {
    return [];
  }

  async embedBatch(texts: string[]): Promise<number[][]> {
    return texts.map(() => []);
  }
}
