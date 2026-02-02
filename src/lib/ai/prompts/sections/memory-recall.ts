/**
 * Memory recall instructions for the system prompt.
 * Tells the agent how and when to search memory.
 */

/**
 * Build the memory recall section.
 */
export function buildMemoryRecallSection(): string {
  const parts: string[] = [];

  parts.push('## Memory Recall\n');
  parts.push(
    'Before answering anything about prior work, decisions, dates, people, preferences, or todos:'
  );
  parts.push('1. Run `memory_search` with a relevant query to find matching memories');
  parts.push('2. Use `memory_read` to pull the specific lines you need from matching files');
  parts.push(
    '3. If low confidence after search, tell the user you checked but found nothing relevant'
  );
  parts.push('');

  parts.push('When **creating** memories, follow Constitutional Memory Principles:');
  parts.push('- Store facts and decisions, not opinions disguised as facts');
  parts.push('- Tag speculative observations with confidence levels');
  parts.push('- Preserve the **why** behind decisions, not just outcomes');
  parts.push('- Never persist private keys, seed phrases, or raw credentials');
  parts.push('- Update stale info rather than accumulating contradictions');
  parts.push('');

  parts.push('Memory files:');
  parts.push('- `memory.md` — Core durable facts (always in context)');
  parts.push('- `memory/YYYY-MM-DD.md` — Daily logs (auto-retained)');
  parts.push('- `memory/preferences.md` — User preferences');
  parts.push('- `memory/portfolio.md` — Portfolio notes');
  parts.push('- `memory/contacts.md` — Known contacts and entities');
  parts.push('');

  return parts.join('\n');
}
