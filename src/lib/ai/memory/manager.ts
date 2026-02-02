import { invoke } from '@tauri-apps/api/core';

import type { EmbeddingProvider } from '../providers/embeddings';
import { chunkMarkdown, sha256 } from './chunker';
import { deduplicateAppend } from './dedup';
import { hybridSearch } from './hybrid-search';
import {
  type ChunkRecordRust,
  DEFAULT_MEMORY_CONFIG,
  type FileRecord,
  MEMORY_PATHS,
  type MemoryConfig,
  type MemorySource,
  type SearchResult,
} from './types';

/**
 * MemoryManager handles indexing, chunking, and searching memory files.
 *
 * Inspired by OpenClaw's MemoryIndexManager, adapted for Tauri:
 * - SQLite operations run in Rust via Tauri commands
 * - Chunking and embedding orchestration in TypeScript
 * - Hybrid search combining FTS5 + vector similarity
 */
export class MemoryManager {
  private config: MemoryConfig;
  private embeddingProvider: EmbeddingProvider | null = null;
  private initialized = false;

  constructor(config: Partial<MemoryConfig> = {}) {
    this.config = { ...DEFAULT_MEMORY_CONFIG, ...config };
  }

  /** Set the embedding provider for vector search */
  setEmbeddingProvider(provider: EmbeddingProvider): void {
    this.embeddingProvider = provider;
  }

  /** Initialize the memory database */
  async init(): Promise<void> {
    await invoke('ai_memory_init');
    this.initialized = true;
  }

  /**
   * Index a memory file: chunk it, compute embeddings, store in SQLite.
   * Only re-indexes chunks that have changed (by hash).
   */
  async indexFile(
    relativePath: string,
    content: string,
    source: MemorySource = 'memory'
  ): Promise<number> {
    if (!this.initialized) await this.init();

    const hash = await sha256(content);
    const now = Date.now();

    // Check if file has changed
    const existingFile = await invoke<FileRecord | null>('ai_memory_get_file', {
      path: relativePath,
    });

    if (existingFile && existingFile.hash === hash) {
      return 0; // No changes
    }

    // Chunk the content
    const chunks = await chunkMarkdown(content, this.config);

    // Delete old chunks for this file
    await invoke('ai_memory_delete_chunks_by_path', { path: relativePath });

    // Store new chunks
    let indexed = 0;
    for (let i = 0; i < chunks.length; i++) {
      const chunk = chunks[i];
      const chunkId = `${relativePath}:${i}:${chunk.hash.slice(0, 8)}`;

      // Compute embedding if provider available
      let embeddingBytes: number[] | null = null;
      if (this.embeddingProvider) {
        try {
          const embedding = await this.embeddingProvider.embedQuery(chunk.text);
          // Store as Float32Array bytes
          const buffer = new Float32Array(embedding).buffer;
          embeddingBytes = Array.from(new Uint8Array(buffer));
        } catch {
          // Embedding failed — store without embedding
        }
      }

      const chunkRecord: ChunkRecordRust = {
        id: chunkId,
        path: relativePath,
        source,
        start_line: chunk.startLine,
        end_line: chunk.endLine,
        hash: chunk.hash,
        model: this.config.embeddingModel,
        text: chunk.text,
        embedding: embeddingBytes,
        updated_at: now,
      };

      await invoke('ai_memory_upsert_chunk', { chunk: chunkRecord });
      indexed++;
    }

    // Update file record
    const fileRecord: FileRecord = {
      path: relativePath,
      source,
      hash,
      mtime: now,
      size: content.length,
    };
    await invoke('ai_memory_upsert_file', { file: fileRecord });

    return indexed;
  }

  /**
   * Index all memory files from the ~/.alphahuman/ directory.
   */
  async indexAll(): Promise<number> {
    let totalIndexed = 0;

    // Index memory.md (core durable facts)
    try {
      const memoryRoot = await invoke<string>('ai_read_memory_file', {
        relativePath: MEMORY_PATHS.MEMORY_ROOT,
      });
      totalIndexed += await this.indexFile(MEMORY_PATHS.MEMORY_ROOT, memoryRoot);
    } catch {
      // memory.md doesn't exist yet — that's fine
    }

    // Index memory directory files
    try {
      const memoryFiles = await invoke<string[]>('ai_list_memory_files', {
        relativeDir: MEMORY_PATHS.MEMORY_DIR,
      });

      for (const fileName of memoryFiles) {
        if (!fileName.endsWith('.md')) continue;
        const relativePath = `${MEMORY_PATHS.MEMORY_DIR}/${fileName}`;
        try {
          const content = await invoke<string>('ai_read_memory_file', { relativePath });
          totalIndexed += await this.indexFile(relativePath, content);
        } catch {
          // Skip unreadable files
        }
      }
    } catch {
      // memory/ directory doesn't exist yet
    }

    return totalIndexed;
  }

  /**
   * Search memory using hybrid FTS5 + vector search.
   */
  async search(query: string): Promise<SearchResult[]> {
    if (!this.initialized) await this.init();
    return hybridSearch(query, this.embeddingProvider, this.config);
  }

  /**
   * Read a specific memory file.
   */
  async readFile(relativePath: string): Promise<string> {
    return invoke<string>('ai_read_memory_file', { relativePath });
  }

  /**
   * Write to a memory file.
   */
  async writeFile(relativePath: string, content: string): Promise<void> {
    await invoke('ai_write_memory_file', { relativePath, content });
    // Re-index the file
    await this.indexFile(relativePath, content);
  }

  /**
   * Append content to a memory file.
   * @param deduplicate When true, filter out lines that already exist in the file.
   */
  async appendToFile(
    relativePath: string,
    content: string,
    options?: { deduplicate?: boolean }
  ): Promise<void> {
    let existing = '';
    try {
      existing = await this.readFile(relativePath);
    } catch {
      // File doesn't exist yet
    }

    let contentToAppend = content;
    if (options?.deduplicate && existing) {
      contentToAppend = deduplicateAppend(existing, content);
      if (!contentToAppend.trim()) {
        return; // All content was duplicate — nothing to write
      }
    }

    const newContent = existing ? `${existing}\n\n${contentToAppend}` : contentToAppend;
    await this.writeFile(relativePath, newContent);
  }

  /**
   * Get today's daily log file path.
   */
  getDailyLogPath(): string {
    const now = new Date();
    const yyyy = now.getFullYear();
    const mm = String(now.getMonth() + 1).padStart(2, '0');
    const dd = String(now.getDate()).padStart(2, '0');
    return `${MEMORY_PATHS.MEMORY_DIR}/${yyyy}-${mm}-${dd}.md`;
  }

  /**
   * Append to today's daily log.
   */
  async appendToDailyLog(content: string): Promise<void> {
    const path = this.getDailyLogPath();
    await this.appendToFile(path, content);
  }

  /** Set metadata */
  async setMeta(key: string, value: string): Promise<void> {
    await invoke('ai_memory_set_meta', { key, value });
  }

  /** Get metadata */
  async getMeta(key: string): Promise<string | null> {
    return invoke<string | null>('ai_memory_get_meta', { key });
  }
}
