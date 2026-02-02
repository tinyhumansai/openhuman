/** Source of a memory file */
export type MemorySource = 'memory' | 'sessions';

/** File metadata tracked in the database */
export interface FileRecord {
  path: string;
  source: MemorySource;
  hash: string;
  mtime: number;
  size: number;
}

/** A chunk of content with optional embedding */
export interface ChunkRecord {
  id: string;
  path: string;
  source: MemorySource;
  startLine: number;
  endLine: number;
  hash: string;
  model: string;
  text: string;
  embedding: number[] | null;
  updatedAt: number;
}

/** Chunk for Rust IPC (uses snake_case and byte arrays) */
export interface ChunkRecordRust {
  id: string;
  path: string;
  source: string;
  start_line: number;
  end_line: number;
  hash: string;
  model: string;
  text: string;
  embedding: number[] | null;
  updated_at: number;
}

/** Search result with relevance score */
export interface SearchResult {
  chunkId: string;
  path: string;
  source: MemorySource;
  text: string;
  score: number;
  startLine: number;
  endLine: number;
  /** Timestamp when the source chunk was last updated */
  updatedAt?: number;
}

/** Configuration for the memory system */
export interface MemoryConfig {
  /** Max tokens per chunk (default: 512) */
  chunkTokenLimit: number;
  /** Overlap tokens between chunks (default: 64) */
  chunkOverlap: number;
  /** Embedding model name */
  embeddingModel: string;
  /** Vector search weight in hybrid search (default: 0.7) */
  vectorWeight: number;
  /** FTS text search weight in hybrid search (default: 0.3) */
  textWeight: number;
  /** Max results for search (default: 10) */
  maxResults: number;
}

export const DEFAULT_MEMORY_CONFIG: MemoryConfig = {
  chunkTokenLimit: 512,
  chunkOverlap: 64,
  embeddingModel: 'text-embedding-3-small',
  vectorWeight: 0.7,
  textWeight: 0.3,
  maxResults: 10,
};

/** Embedding cache entry */
export interface EmbeddingCacheEntry {
  provider: string;
  model: string;
  hash: string;
  embedding: number[];
  dims: number | null;
  updatedAt: number;
}

/** Memory file layout under ~/.alphahuman/ */
export const MEMORY_PATHS = {
  CONSTITUTION: 'CONSTITUTION.md',
  MEMORY_ROOT: 'memory.md',
  MEMORY_DIR: 'memory',
  SESSIONS_DIR: 'sessions',
  SKILLS_DIR: 'skills',
  IDENTITY: 'identity.md',
} as const;
