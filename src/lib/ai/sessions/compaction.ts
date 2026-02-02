import type { ConstitutionConfig } from '../constitution/types';
import type { MemoryManager } from '../memory/manager';
import { buildSystemPrompt } from '../prompts/system-prompt';
import { COMPACTION_SUMMARY_TEMPLATE } from '../prompts/templates';
import type { LLMProvider, Message } from '../providers/interface';
import { executeMemoryFlush } from './memory-flush';
import { compressMessagesForSummary, DEFAULT_TOOL_CAPTURE_CONFIG } from './tool-compress';
import { DEFAULT_SESSION_CONFIG, type SessionConfig, type ToolCaptureConfig } from './types';

/**
 * Estimate token count for messages.
 */
function estimateMessageTokens(messages: Message[]): number {
  let total = 0;
  for (const msg of messages) {
    for (const block of msg.content) {
      if (block.type === 'text') {
        total += Math.ceil(block.text.length / 4);
      } else {
        total += 50; // Rough estimate for tool calls
      }
    }
  }
  return total;
}

/**
 * Check if context compaction is needed based on token count.
 */
export function shouldCompact(messages: Message[], config: Partial<SessionConfig> = {}): boolean {
  const { maxContextTokens } = { ...DEFAULT_SESSION_CONFIG, ...config };
  return estimateMessageTokens(messages) > maxContextTokens;
}

/**
 * Compact the session context.
 *
 * Steps:
 * 1. Run memory flush (save durable facts before discarding)
 * 2. Summarize old messages into a compact context block
 * 3. Keep recent messages + summary
 */
export async function compactSession(params: {
  provider: LLMProvider;
  constitution: ConstitutionConfig;
  memoryManager: MemoryManager;
  messages: Message[];
  compactionCount: number;
  lastFlushCompactionCount?: number;
  config?: Partial<SessionConfig>;
  toolCaptureConfig?: ToolCaptureConfig;
}): Promise<{
  compactedMessages: Message[];
  summary: string;
  compactionCount: number;
  memoryFlushCompactionCount: number;
}> {
  const {
    provider,
    constitution,
    memoryManager,
    messages,
    compactionCount,
    lastFlushCompactionCount,
    config = {},
    toolCaptureConfig = DEFAULT_TOOL_CAPTURE_CONFIG,
  } = params;

  const { preserveRecentTokens, memoryFlushEnabled } = { ...DEFAULT_SESSION_CONFIG, ...config };

  const newCompactionCount = compactionCount + 1;

  // Step 1: Memory flush (with compressed messages to save tokens)
  if (memoryFlushEnabled) {
    const compressedForFlush = compressMessagesForSummary(messages, toolCaptureConfig);
    await executeMemoryFlush({
      provider,
      constitution,
      memoryManager,
      conversationMessages: compressedForFlush,
      currentCompactionCount: newCompactionCount,
      lastFlushCompactionCount,
    });
  }

  // Step 2: Split messages into old (to summarize) and recent (to keep)
  let recentTokens = 0;
  let splitIndex = messages.length;
  for (let i = messages.length - 1; i >= 0; i--) {
    const msgTokens = estimateMessageTokens([messages[i]]);
    if (recentTokens + msgTokens > preserveRecentTokens) break;
    recentTokens += msgTokens;
    splitIndex = i;
  }

  const oldMessages = messages.slice(0, splitIndex);
  const recentMessages = messages.slice(splitIndex);

  // Step 3: Summarize old messages (compress tool output before sending to LLM)
  const compressedOldMessages = compressMessagesForSummary(oldMessages, toolCaptureConfig);
  const systemPrompt = buildSystemPrompt({ constitution, mode: 'minimal' });

  const summaryResponse = await provider.complete({
    systemPrompt,
    messages: [
      ...compressedOldMessages,
      { role: 'user', content: [{ type: 'text', text: COMPACTION_SUMMARY_TEMPLATE }] },
    ],
    maxTokens: 2048,
  });

  const summary =
    summaryResponse.content.find(b => b.type === 'text')?.text ||
    'Previous conversation context was compacted.';

  // Step 4: Build compacted message list
  const compactedMessages: Message[] = [
    {
      role: 'system',
      content: [
        {
          type: 'text',
          text: `[Context from previous conversation (compaction #${newCompactionCount})]\n\n${summary}`,
        },
      ],
    },
    ...recentMessages,
  ];

  return {
    compactedMessages,
    summary,
    compactionCount: newCompactionCount,
    memoryFlushCompactionCount: newCompactionCount,
  };
}
