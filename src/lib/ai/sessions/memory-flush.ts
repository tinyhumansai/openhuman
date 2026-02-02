import type { ConstitutionConfig } from '../constitution/types';
import { validateMemoryContent } from '../constitution/validator';
import type { MemoryManager } from '../memory/manager';
import { buildSystemPrompt } from '../prompts/system-prompt';
import { MEMORY_FLUSH_TEMPLATE } from '../prompts/templates';
import type { LLMProvider, Message } from '../providers/interface';

/**
 * Execute a memory flush before context compaction.
 *
 * The memory flush prompts the LLM to extract durable facts from the
 * conversation and write them to appropriate memory files, following
 * Constitutional Memory Principles.
 */
export async function executeMemoryFlush(params: {
  provider: LLMProvider;
  constitution: ConstitutionConfig;
  memoryManager: MemoryManager;
  conversationMessages: Message[];
  currentCompactionCount: number;
  lastFlushCompactionCount?: number;
}): Promise<{ flushed: boolean; savedFiles: string[] }> {
  const {
    provider,
    constitution,
    memoryManager,
    conversationMessages,
    currentCompactionCount,
    lastFlushCompactionCount,
  } = params;

  // Don't flush twice for the same compaction
  if (
    lastFlushCompactionCount !== undefined &&
    lastFlushCompactionCount >= currentCompactionCount
  ) {
    return { flushed: false, savedFiles: [] };
  }

  // Build a minimal system prompt for the flush
  const systemPrompt = buildSystemPrompt({ constitution, mode: 'minimal' });

  // Create flush request with conversation context
  const flushMessages: Message[] = [
    ...conversationMessages,
    { role: 'user', content: [{ type: 'text', text: MEMORY_FLUSH_TEMPLATE }] },
  ];

  // Get the LLM to identify what should be saved
  const response = await provider.complete({
    systemPrompt,
    messages: flushMessages,
    maxTokens: 2048,
  });

  const savedFiles: string[] = [];

  // Parse the response for memory write instructions
  for (const block of response.content) {
    if (block.type !== 'text') continue;

    const text = block.text;

    // Look for structured memory entries in the response
    // The LLM is expected to format them as:
    // FILE: <path>
    // CONTENT: <content>
    const fileBlocks = text.split(/(?=FILE:\s)/);
    for (const fb of fileBlocks) {
      const fileMatch = fb.match(/FILE:\s*(.+?)[\n\r]/);
      const contentMatch = fb.match(/CONTENT:\s*([\s\S]+?)(?=FILE:|$)/);

      if (fileMatch && contentMatch) {
        const filePath = fileMatch[1].trim();
        const content = contentMatch[1].trim();

        // Validate against constitution before writing
        const validation = validateMemoryContent(content, constitution);
        if (!validation.valid) {
          continue; // Skip content that violates constitutional rules
        }

        try {
          await memoryManager.appendToFile(filePath, content, { deduplicate: true });
          savedFiles.push(filePath);
        } catch {
          // File write failed — non-fatal
        }
      }
    }

    // If no structured format, save the whole response to daily log
    if (savedFiles.length === 0 && text.trim()) {
      const validation = validateMemoryContent(text, constitution);
      if (validation.valid) {
        try {
          await memoryManager.appendToDailyLog(
            `## Memory Flush (Compaction #${currentCompactionCount})\n\n${text}`
          );
          savedFiles.push(memoryManager.getDailyLogPath());
        } catch {
          // Non-fatal
        }
      }
    }
  }

  return { flushed: true, savedFiles };
}
