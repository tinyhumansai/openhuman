import { DEFAULT_MEMORY_CONFIG, type MemoryConfig } from './types';

/** A chunk of markdown content */
export interface MarkdownChunk {
  text: string;
  startLine: number;
  endLine: number;
  hash: string;
}

/**
 * Estimate token count from text.
 * Rough approximation: ~4 characters per token for English text.
 */
function estimateTokens(text: string): number {
  return Math.ceil(text.length / 4);
}

/**
 * Compute SHA-256 hash of a string.
 * Uses the Web Crypto API available in modern browsers and Tauri.
 */
export async function sha256(text: string): Promise<string> {
  const encoder = new TextEncoder();
  const data = encoder.encode(text);
  const hashBuffer = await crypto.subtle.digest('SHA-256', data);
  const hashArray = Array.from(new Uint8Array(hashBuffer));
  return hashArray.map(b => b.toString(16).padStart(2, '0')).join('');
}

/**
 * Synchronous hash for non-async contexts.
 * Simple FNV-1a hash (not cryptographic, but fast for chunk IDs).
 */
export function quickHash(text: string): string {
  let hash = 2166136261;
  for (let i = 0; i < text.length; i++) {
    hash ^= text.charCodeAt(i);
    hash = (hash * 16777619) >>> 0;
  }
  return hash.toString(16).padStart(8, '0');
}

/**
 * Chunk markdown content into overlapping pieces.
 *
 * Strategy:
 * 1. Split by markdown headers (##, ###) as natural boundaries
 * 2. If a section exceeds the token limit, split by paragraphs
 * 3. If a paragraph exceeds the limit, split by sentences
 * 4. Apply overlap between chunks for context preservation
 */
export async function chunkMarkdown(
  content: string,
  config: Partial<MemoryConfig> = {}
): Promise<MarkdownChunk[]> {
  const { chunkTokenLimit, chunkOverlap } = { ...DEFAULT_MEMORY_CONFIG, ...config };

  const lines = content.split('\n');
  const chunks: MarkdownChunk[] = [];

  let currentChunkLines: string[] = [];
  let currentStartLine = 0;
  let currentTokens = 0;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const lineTokens = estimateTokens(line);

    // Check if this line starts a new header section
    const isHeader = /^#{1,4}\s/.test(line);

    // If adding this line would exceed the limit, or we hit a header
    // and already have content, flush the current chunk
    if (
      (currentTokens + lineTokens > chunkTokenLimit && currentChunkLines.length > 0) ||
      (isHeader && currentTokens > chunkTokenLimit * 0.3 && currentChunkLines.length > 0)
    ) {
      const text = currentChunkLines.join('\n');
      chunks.push({ text, startLine: currentStartLine, endLine: i - 1, hash: await sha256(text) });

      // Apply overlap: keep the last N tokens worth of lines
      const overlapLines: string[] = [];
      let overlapTokens = 0;
      for (let j = currentChunkLines.length - 1; j >= 0; j--) {
        const lt = estimateTokens(currentChunkLines[j]);
        if (overlapTokens + lt > chunkOverlap * 4) break;
        overlapLines.unshift(currentChunkLines[j]);
        overlapTokens += lt;
      }

      currentChunkLines = overlapLines;
      currentStartLine = i - overlapLines.length;
      currentTokens = overlapTokens;
    }

    currentChunkLines.push(line);
    currentTokens += lineTokens;
  }

  // Flush remaining content
  if (currentChunkLines.length > 0) {
    const text = currentChunkLines.join('\n');
    chunks.push({
      text,
      startLine: currentStartLine,
      endLine: lines.length - 1,
      hash: await sha256(text),
    });
  }

  return chunks;
}
