/**
 * User context section of the system prompt.
 * Injects preferences, timezone, and project-specific context.
 */
import { type EnrichedSearchResult, formatMemoryContext } from '../../memory/context-formatter';

export interface MemoryCategorized {
  profileFacts: string[];
  recentContext: string[];
  searchResults: EnrichedSearchResult[];
}

export interface UserContext {
  /** User's timezone (IANA format) */
  timezone?: string;
  /** User display name */
  displayName?: string;
  /** User preferences loaded from memory */
  preferences?: string;
  /** Content of memory.md (always in context) */
  memoryContext?: string;
  /** Content of identity.md */
  identityContext?: string;
  /** Categorized memory context (takes precedence over raw memoryContext) */
  memoryCategorized?: MemoryCategorized;
}

/**
 * Build the user context section.
 */
export function buildContextSection(context: UserContext): string {
  const parts: string[] = [];

  if (context.displayName || context.timezone) {
    parts.push('## User Context\n');
    if (context.displayName) {
      parts.push(`- **User**: ${context.displayName}`);
    }
    if (context.timezone) {
      parts.push(`- **Timezone**: ${context.timezone}`);
    }
    parts.push('');
  }

  // Use categorized memory if available, otherwise fall back to raw
  if (context.memoryCategorized) {
    const formatted = formatMemoryContext(context.memoryCategorized);
    if (formatted) {
      parts.push(formatted);
      parts.push('');
    }
  } else {
    if (context.preferences) {
      parts.push('## User Preferences\n');
      parts.push(context.preferences);
      parts.push('');
    }

    if (context.memoryContext) {
      parts.push('## Project Context (memory.md)\n');
      parts.push(context.memoryContext);
      parts.push('');
    }
  }

  if (context.identityContext) {
    parts.push('## Agent Persona (identity.md)\n');
    parts.push(context.identityContext);
    parts.push('');
  }

  return parts.join('\n');
}
