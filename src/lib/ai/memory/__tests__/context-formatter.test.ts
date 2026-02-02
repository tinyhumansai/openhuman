import { describe, expect, it } from 'vitest';

import {
  deduplicateMemories,
  type EnrichedSearchResult,
  formatMemoryContext,
  formatRelativeTime,
} from '../context-formatter';

describe('formatRelativeTime', () => {
  it("should return 'just now' for timestamps within 30 minutes", () => {
    const fiveMinAgo = Date.now() - 5 * 60 * 1000;
    expect(formatRelativeTime(fiveMinAgo)).toBe('just now');
  });

  it('should return minutes ago for 30-60 minutes', () => {
    const fortyMinAgo = Date.now() - 40 * 60 * 1000;
    expect(formatRelativeTime(fortyMinAgo)).toBe('40mins ago');
  });

  it('should return hours ago for 1-24 hours', () => {
    const threeHoursAgo = Date.now() - 3 * 60 * 60 * 1000;
    expect(formatRelativeTime(threeHoursAgo)).toBe('3hrs ago');
  });

  it('should return days ago for 1-7 days', () => {
    const twoDaysAgo = Date.now() - 2 * 24 * 60 * 60 * 1000;
    expect(formatRelativeTime(twoDaysAgo)).toBe('2d ago');
  });

  it('should return date for older timestamps in same year', () => {
    const now = new Date();
    const jan1 = new Date(now.getFullYear(), 0, 15);
    // Only test if Jan 15 is more than 7 days ago
    if (Date.now() - jan1.getTime() > 7 * 24 * 60 * 60 * 1000) {
      const result = formatRelativeTime(jan1.getTime());
      expect(result).toContain('Jan');
      expect(result).toContain('15');
    }
  });

  it('should return empty string for invalid timestamps', () => {
    expect(formatRelativeTime(NaN)).toBe('');
  });
});

describe('deduplicateMemories', () => {
  it('should remove exact duplicates within profile facts', () => {
    const result = deduplicateMemories(['fact A', 'fact B', 'fact A'], [], []);
    expect(result.profile).toEqual(['fact A', 'fact B']);
  });

  it('should remove profile facts from recent context', () => {
    const result = deduplicateMemories(
      ['User likes dark mode'],
      ['User likes dark mode', 'Session started'],
      []
    );
    expect(result.profile).toEqual(['User likes dark mode']);
    expect(result.recent).toEqual(['Session started']);
  });

  it('should remove higher-tier items from search results', () => {
    const searchResults: EnrichedSearchResult[] = [
      {
        chunkId: 'c1',
        path: 'memory.md',
        source: 'memory',
        text: 'User likes dark mode',
        score: 0.9,
        startLine: 0,
        endLine: 0,
      },
      {
        chunkId: 'c2',
        path: 'memory.md',
        source: 'memory',
        text: 'User prefers TypeScript',
        score: 0.8,
        startLine: 1,
        endLine: 1,
      },
    ];

    const result = deduplicateMemories(['User likes dark mode'], [], searchResults);
    expect(result.search).toHaveLength(1);
    expect(result.search[0].text).toBe('User prefers TypeScript');
  });

  it('should normalize whitespace for comparison', () => {
    const result = deduplicateMemories(['  fact  A  '], ['fact A'], []);
    expect(result.recent).toHaveLength(0);
  });

  it('should be case-insensitive', () => {
    const result = deduplicateMemories(['User Likes Dark Mode'], ['user likes dark mode'], []);
    expect(result.recent).toHaveLength(0);
  });
});

describe('formatMemoryContext', () => {
  it('should return null when all arrays are empty', () => {
    const result = formatMemoryContext({ profileFacts: [], recentContext: [], searchResults: [] });
    expect(result).toBeNull();
  });

  it('should render profile facts section', () => {
    const result = formatMemoryContext({
      profileFacts: ['Prefers dark mode', 'Uses TypeScript'],
      recentContext: [],
      searchResults: [],
    });
    expect(result).toContain('<memory-context>');
    expect(result).toContain('## User Profile (Persistent)');
    expect(result).toContain('- Prefers dark mode');
    expect(result).toContain('- Uses TypeScript');
    expect(result).toContain('</memory-context>');
  });

  it('should render recent context section', () => {
    const result = formatMemoryContext({
      profileFacts: [],
      recentContext: ['Discussed API design', 'Reviewed PR #42'],
      searchResults: [],
    });
    expect(result).toContain('## Recent Context');
    expect(result).toContain('- Discussed API design');
  });

  it('should render search results with scores', () => {
    const result = formatMemoryContext({
      profileFacts: [],
      recentContext: [],
      searchResults: [
        {
          chunkId: 'c1',
          path: 'memory.md',
          source: 'memory',
          text: 'User prefers vim keybindings',
          score: 0.87,
          startLine: 0,
          endLine: 0,
          updatedAt: Date.now() - 2 * 60 * 60 * 1000,
        },
      ],
    });
    expect(result).toContain('## Relevant Memories');
    expect(result).toContain('[87%]');
    expect(result).toContain('2hrs ago');
  });

  it('should deduplicate across tiers', () => {
    const result = formatMemoryContext({
      profileFacts: ['Likes dark mode'],
      recentContext: ['Likes dark mode'],
      searchResults: [],
    });
    // "Likes dark mode" should appear only once (in profile)
    const matches = result!.match(/Likes dark mode/g);
    expect(matches).toHaveLength(1);
  });

  it('should respect maxResults', () => {
    const facts = Array.from({ length: 20 }, (_, i) => `Fact ${i}`);
    const result = formatMemoryContext({
      profileFacts: facts,
      recentContext: [],
      searchResults: [],
      maxResults: 5,
    });
    const bulletCount = result!.match(/^- /gm)?.length ?? 0;
    expect(bulletCount).toBe(5);
  });

  it('should include intro and disclaimer', () => {
    const result = formatMemoryContext({
      profileFacts: ['Test fact'],
      recentContext: [],
      searchResults: [],
    });
    expect(result).toContain('recalled context about the user');
    expect(result).toContain("don't force them into every response");
  });

  it('should truncate long search result text', () => {
    const longText = 'a'.repeat(300);
    const result = formatMemoryContext({
      profileFacts: [],
      recentContext: [],
      searchResults: [
        {
          chunkId: 'c1',
          path: 'test.md',
          source: 'memory',
          text: longText,
          score: 0.9,
          startLine: 0,
          endLine: 0,
        },
      ],
    });
    expect(result).toContain('...');
    // The truncated text + formatting should be shorter than the full text
    expect(result!.length).toBeLessThan(longText.length + 200);
  });
});
