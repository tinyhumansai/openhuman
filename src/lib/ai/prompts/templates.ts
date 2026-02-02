/**
 * Prompt templates for common AI interactions.
 */

/** Template for memory flush before compaction */
export const MEMORY_FLUSH_TEMPLATE = `Pre-compaction memory flush. Review the conversation so far and store any durable memories now, following Constitutional Memory Principles.

Store to the appropriate memory files:
- Important facts, decisions, and their reasoning → memory.md or daily log
- User preferences or settings → memory/preferences.md
- Portfolio-related notes → memory/portfolio.md
- Contact/entity information → memory/contacts.md

Rules:
- Only store facts and decisions, not opinions
- Tag speculative observations with confidence levels
- Never persist private keys, seed phrases, or credentials
- Preserve the "why" behind decisions
- Update stale info rather than accumulate contradictions`;

/** Template for session compaction summary */
export const COMPACTION_SUMMARY_TEMPLATE = `Summarize the conversation so far into a compact context block. Include:
1. Key decisions made and their reasoning
2. Important facts discussed
3. Current task state and next steps
4. Any risk warnings or disclaimers that were given
5. User preferences expressed during the conversation

Keep the summary concise but preserve critical context. This will replace the full history.`;

/** Template for daily log entry */
export const DAILY_LOG_TEMPLATE = (date: string) => `# Daily Log — ${date}\n\n`;

/** Silent response token (matches OpenClaw) */
export const SILENT_TOKEN = '\u2039silent\u203a';
