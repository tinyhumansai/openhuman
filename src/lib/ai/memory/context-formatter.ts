import type { SearchResult } from './types';

/** Enriched search result with timestamp for display */
export interface EnrichedSearchResult extends SearchResult {
  updatedAt?: number;
}

/**
 * Format a timestamp as a human-readable relative time string.
 * Ported from claude-supermemory's format-context.js.
 */
export function formatRelativeTime(timestamp: number): string {
  try {
    if (!Number.isFinite(timestamp)) return '';

    const now = Date.now();
    const seconds = (now - timestamp) / 1000;
    const minutes = seconds / 60;
    const hours = seconds / 3600;
    const days = seconds / 86400;

    if (minutes < 30) return 'just now';
    if (minutes < 60) return `${Math.floor(minutes)}mins ago`;
    if (hours < 24) return `${Math.floor(hours)}hrs ago`;
    if (days < 7) return `${Math.floor(days)}d ago`;

    const dt = new Date(timestamp);
    const month = dt.toLocaleString('en', { month: 'short' });
    if (dt.getFullYear() === new Date().getFullYear()) {
      return `${dt.getDate()} ${month}`;
    }
    return `${dt.getDate()} ${month}, ${dt.getFullYear()}`;
  } catch {
    return '';
  }
}

/**
 * Deduplicate memories across tiers with priority: profile > recent > search.
 * Items seen in a higher-priority tier are removed from lower tiers.
 */
export function deduplicateMemories(
  profileFacts: string[],
  recentContext: string[],
  searchResults: EnrichedSearchResult[]
): { profile: string[]; recent: string[]; search: EnrichedSearchResult[] } {
  const seen = new Set<string>();

  const normalize = (s: string) => s.trim().toLowerCase().replace(/\s+/g, ' ');

  const uniqueProfile = profileFacts.filter(fact => {
    const key = normalize(fact);
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });

  const uniqueRecent = recentContext.filter(fact => {
    const key = normalize(fact);
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });

  const uniqueSearch = searchResults.filter(r => {
    const key = normalize(r.text);
    if (!key || seen.has(key)) return false;
    seen.add(key);
    return true;
  });

  return { profile: uniqueProfile, recent: uniqueRecent, search: uniqueSearch };
}

/**
 * Format categorized memory context for injection into the system prompt.
 *
 * Renders three tiers:
 * 1. User Profile (Persistent) — stable facts from memory.md / preferences
 * 2. Recent Context — today's daily log entries
 * 3. Relevant Memories — search results with timestamps and similarity scores
 */
export function formatMemoryContext(params: {
  profileFacts: string[];
  recentContext: string[];
  searchResults: EnrichedSearchResult[];
  maxResults?: number;
}): string | null {
  const { maxResults = 10 } = params;

  const deduped = deduplicateMemories(
    params.profileFacts,
    params.recentContext,
    params.searchResults
  );

  const profile = deduped.profile.slice(0, maxResults);
  const recent = deduped.recent.slice(0, maxResults);
  const search = deduped.search.slice(0, maxResults);

  if (profile.length === 0 && recent.length === 0 && search.length === 0) {
    return null;
  }

  const sections: string[] = [];

  if (profile.length > 0) {
    sections.push('## User Profile (Persistent)\n' + profile.map(f => `- ${f}`).join('\n'));
  }

  if (recent.length > 0) {
    sections.push('## Recent Context\n' + recent.map(f => `- ${f}`).join('\n'));
  }

  if (search.length > 0) {
    const lines = search.map(r => {
      const timeStr = r.updatedAt ? formatRelativeTime(r.updatedAt) : '';
      const pct = `[${Math.round(r.score * 100)}%]`;
      const prefix = timeStr ? `[${timeStr}] ` : '';
      // Truncate long text for context display
      const text = r.text.length > 200 ? r.text.slice(0, 200) + '...' : r.text;
      return `- ${prefix}${text} ${pct}`;
    });
    sections.push('## Relevant Memories\n' + lines.join('\n'));
  }

  const intro =
    'The following is recalled context about the user. Reference it only when relevant to the conversation.';
  const disclaimer =
    "Use these memories naturally when relevant but don't force them into every response or make assumptions beyond what's stated.";

  return `<memory-context>\n${intro}\n\n${sections.join('\n\n')}\n\n${disclaimer}\n</memory-context>`;
}
