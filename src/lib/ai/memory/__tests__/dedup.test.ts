import { describe, expect, it } from 'vitest';

import { deduplicateAppend, isDuplicateFact } from '../dedup';

describe('isDuplicateFact', () => {
  it('should detect exact duplicates', () => {
    const existing = '- User prefers dark mode\n- User uses TypeScript';
    expect(isDuplicateFact(existing, 'User prefers dark mode')).toBe(true);
  });

  it('should detect case-insensitive duplicates', () => {
    const existing = '- User Prefers Dark Mode';
    expect(isDuplicateFact(existing, 'user prefers dark mode')).toBe(true);
  });

  it('should detect substrings of existing lines', () => {
    const existing = '- User prefers dark mode with custom theme settings';
    expect(isDuplicateFact(existing, 'dark mode')).toBe(true);
  });

  it('should NOT flag more detailed versions as duplicates', () => {
    const existing = '- User prefers dark mode';
    expect(
      isDuplicateFact(existing, 'User prefers dark mode with custom syntax highlighting')
    ).toBe(false);
  });

  it('should flag empty facts as duplicates', () => {
    expect(isDuplicateFact('some content', '')).toBe(true);
    expect(isDuplicateFact('some content', '  ')).toBe(true);
  });

  it('should handle whitespace differences', () => {
    const existing = '- User  prefers  dark  mode';
    expect(isDuplicateFact(existing, 'User prefers dark mode')).toBe(true);
  });

  it('should return false for genuinely new facts', () => {
    const existing = '- User prefers dark mode\n- Uses TypeScript';
    expect(isDuplicateFact(existing, 'User lives in Berlin')).toBe(false);
  });

  it('should handle empty existing content', () => {
    expect(isDuplicateFact('', 'new fact')).toBe(false);
  });
});

describe('deduplicateAppend', () => {
  it('should filter out duplicate lines', () => {
    const existing = '- User prefers dark mode\n- User uses TypeScript';
    const newContent = '- User prefers dark mode\n- User lives in Berlin';
    const result = deduplicateAppend(existing, newContent);
    expect(result).toContain('User lives in Berlin');
    expect(result).not.toContain('User prefers dark mode');
  });

  it('should keep headers and formatting', () => {
    const existing = '# Memory\n\n- Fact 1';
    const newContent = '## New Section\n\n- Fact 1\n- Fact 2';
    const result = deduplicateAppend(existing, newContent);
    expect(result).toContain('## New Section');
    expect(result).toContain('Fact 2');
    expect(result).not.toMatch(/^- Fact 1$/m);
  });

  it('should keep separator lines', () => {
    const existing = '- Old fact';
    const newContent = '---\n- New fact';
    const result = deduplicateAppend(existing, newContent);
    expect(result).toContain('---');
    expect(result).toContain('New fact');
  });

  it('should return empty string when all content is duplicate', () => {
    const existing = '- User prefers dark mode\n- User uses TypeScript';
    const newContent = '- User prefers dark mode\n- User uses TypeScript';
    const result = deduplicateAppend(existing, newContent);
    expect(result).toBe('');
  });

  it('should handle bullet-point markers in new content', () => {
    const existing = 'User prefers dark mode';
    const newContent = '- User prefers dark mode\n* User lives in Berlin';
    const result = deduplicateAppend(existing, newContent);
    expect(result).not.toContain('dark mode');
    expect(result).toContain('User lives in Berlin');
  });

  it('should handle numbered list markers', () => {
    const existing = 'User prefers dark mode';
    const newContent = '1. User prefers dark mode\n2. User lives in Berlin';
    const result = deduplicateAppend(existing, newContent);
    expect(result).toContain('User lives in Berlin');
  });

  it('should preserve empty lines between novel content', () => {
    const existing = '- Old fact';
    const newContent = '- New fact 1\n\n- New fact 2';
    const result = deduplicateAppend(existing, newContent);
    expect(result).toContain('New fact 1');
    expect(result).toContain('New fact 2');
  });
});
