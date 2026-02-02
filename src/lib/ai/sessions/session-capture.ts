import type { ConstitutionConfig } from '../constitution/types';
import type { MemoryManager } from '../memory/manager';
import type { LLMProvider, Message } from '../providers/interface';
import { executeMemoryFlush } from './memory-flush';
import { compressMessagesForSummary, DEFAULT_TOOL_CAPTURE_CONFIG } from './tool-compress';
import type { SessionEntry, ToolCaptureConfig } from './types';

/**
 * Estimate token count for a message (rough: ~4 chars per token).
 */
function estimateTokens(text: string): number {
  return Math.ceil(text.length / 4);
}

/**
 * Count user turns in a message array.
 */
function countUserTurns(messages: Message[]): number {
  return messages.filter(m => m.role === 'user').length;
}

/**
 * Estimate total user content tokens.
 */
function estimateUserTokens(messages: Message[]): number {
  let total = 0;
  for (const msg of messages) {
    if (msg.role !== 'user') continue;
    for (const block of msg.content) {
      if (block.type === 'text') {
        total += estimateTokens(block.text);
      }
    }
  }
  return total;
}

/**
 * Determine if a session has enough substance to warrant a memory capture.
 *
 * Criteria:
 * - At least 2 user turns
 * - At least 100 estimated tokens of user content
 * - Session hasn't been flushed recently (no flush at current compaction count)
 */
export function shouldCaptureSession(messages: Message[], entry: SessionEntry): boolean {
  if (countUserTurns(messages) < 2) return false;
  if (estimateUserTokens(messages) < 100) return false;

  // If memory was already flushed at or after the current compaction count,
  // the session has been captured recently
  if (
    entry.memoryFlushCompactionCount !== undefined &&
    entry.memoryFlushCompactionCount >= entry.compactionCount
  ) {
    return false;
  }

  return true;
}

/**
 * Run a lightweight memory flush at session end.
 *
 * Compresses messages first (to save tokens), then reuses the
 * existing executeMemoryFlush() pipeline.
 */
export async function captureSessionEnd(params: {
  provider: LLMProvider;
  constitution: ConstitutionConfig;
  memoryManager: MemoryManager;
  messages: Message[];
  sessionId: string;
  sessionEntry: SessionEntry;
  toolCaptureConfig?: ToolCaptureConfig;
}): Promise<{ captured: boolean; savedFiles: string[] }> {
  const {
    provider,
    constitution,
    memoryManager,
    messages,
    sessionEntry,
    toolCaptureConfig = DEFAULT_TOOL_CAPTURE_CONFIG,
  } = params;

  // Compress messages to reduce token usage for the flush
  const compressed = compressMessagesForSummary(messages, toolCaptureConfig);

  const result = await executeMemoryFlush({
    provider,
    constitution,
    memoryManager,
    conversationMessages: compressed,
    currentCompactionCount: sessionEntry.compactionCount + 1,
    lastFlushCompactionCount: sessionEntry.memoryFlushCompactionCount,
  });

  return { captured: result.flushed, savedFiles: result.savedFiles };
}
