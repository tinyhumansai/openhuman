import type { EnrichedSearchResult } from './context-formatter';
import type { MemoryManager } from './manager';
import { MEMORY_PATHS } from './types';

export interface CategorizedContext {
  profileFacts: string[];
  recentContext: string[];
  searchResults: EnrichedSearchResult[];
}

/**
 * Parse bullet-point lines from a markdown file into an array of facts.
 * Strips leading `- `, `* `, or numbered list markers.
 */
function parseBulletPoints(content: string): string[] {
  return content
    .split('\n')
    .map(line => line.trim())
    .filter(line => line.length > 0)
    .map(line => line.replace(/^[-*]\s+/, '').replace(/^\d+\.\s+/, ''))
    .filter(line => line.length > 0 && !line.startsWith('#'));
}

/**
 * Load categorized memory context from the user's memory files.
 *
 * 1. Reads memory.md -> parses into profileFacts[]
 * 2. Reads memory/preferences.md -> merges into profileFacts[]
 * 3. Reads today's daily log -> parses into recentContext[]
 * 4. Optionally runs a search -> searchResults[]
 */
export async function loadCategorizedContext(
  memoryManager: MemoryManager,
  query?: string
): Promise<CategorizedContext> {
  const profileFacts: string[] = [];
  const recentContext: string[] = [];
  let searchResults: EnrichedSearchResult[] = [];

  // 1. Read memory.md (core durable facts)
  try {
    const memoryRoot = await memoryManager.readFile(MEMORY_PATHS.MEMORY_ROOT);
    profileFacts.push(...parseBulletPoints(memoryRoot));
  } catch {
    // memory.md doesn't exist yet
  }

  // 2. Read memory/preferences.md
  try {
    const prefs = await memoryManager.readFile(`${MEMORY_PATHS.MEMORY_DIR}/preferences.md`);
    profileFacts.push(...parseBulletPoints(prefs));
  } catch {
    // preferences.md doesn't exist yet
  }

  // 3. Read today's daily log
  try {
    const dailyLogPath = memoryManager.getDailyLogPath();
    const dailyLog = await memoryManager.readFile(dailyLogPath);
    recentContext.push(...parseBulletPoints(dailyLog));
  } catch {
    // No daily log for today
  }

  // 4. Optional search
  if (query) {
    try {
      const results = await memoryManager.search(query);
      searchResults = results.map(r => ({
        ...r,
        updatedAt: (r as EnrichedSearchResult).updatedAt,
      }));
    } catch {
      // Search failed — non-fatal
    }
  }

  return { profileFacts, recentContext, searchResults };
}
