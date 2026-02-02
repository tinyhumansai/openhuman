import { describe, expect, it } from 'vitest';

import { chunkMarkdown, quickHash, sha256 } from '../chunker';

describe('sha256', () => {
  it('should produce consistent hashes', async () => {
    const hash1 = await sha256('hello world');
    const hash2 = await sha256('hello world');
    expect(hash1).toBe(hash2);
  });

  it('should produce different hashes for different inputs', async () => {
    const hash1 = await sha256('hello');
    const hash2 = await sha256('world');
    expect(hash1).not.toBe(hash2);
  });

  it('should produce 64-character hex strings', async () => {
    const hash = await sha256('test');
    expect(hash).toHaveLength(64);
    expect(hash).toMatch(/^[0-9a-f]{64}$/);
  });

  it('should handle empty string', async () => {
    const hash = await sha256('');
    expect(hash).toHaveLength(64);
  });
});

describe('quickHash', () => {
  it('should produce consistent hashes', () => {
    expect(quickHash('test')).toBe(quickHash('test'));
  });

  it('should produce different hashes for different inputs', () => {
    expect(quickHash('hello')).not.toBe(quickHash('world'));
  });

  it('should return a hex string', () => {
    expect(quickHash('test')).toMatch(/^[0-9a-f]+$/);
  });
});

describe('chunkMarkdown', () => {
  it('should chunk small content into a single chunk', async () => {
    const content = '# Hello\n\nThis is a small document.';
    const chunks = await chunkMarkdown(content);
    expect(chunks).toHaveLength(1);
    expect(chunks[0].text).toBe(content);
    expect(chunks[0].startLine).toBe(0);
    expect(chunks[0].endLine).toBe(2);
  });

  it('should produce hashes for each chunk', async () => {
    const content = '# Test\n\nSome content here.';
    const chunks = await chunkMarkdown(content);
    expect(chunks[0].hash).toHaveLength(64);
    expect(chunks[0].hash).toMatch(/^[0-9a-f]{64}$/);
  });

  it('should split large content into multiple chunks', async () => {
    // Create content that exceeds default chunk size (512 tokens ~ 2048 chars)
    const lines = [];
    for (let i = 0; i < 200; i++) {
      lines.push(`Line ${i}: This is a line of content with some filler text to increase length.`);
    }
    const content = lines.join('\n');
    const chunks = await chunkMarkdown(content, { chunkTokenLimit: 128 });
    expect(chunks.length).toBeGreaterThan(1);
  });

  it('should split on markdown headers when possible', async () => {
    const content = [
      '## Section 1',
      '',
      'Content for section 1. '.repeat(100),
      '',
      '## Section 2',
      '',
      'Content for section 2. '.repeat(100),
    ].join('\n');

    const chunks = await chunkMarkdown(content, { chunkTokenLimit: 256 });
    expect(chunks.length).toBeGreaterThanOrEqual(2);
  });

  it('should handle empty content', async () => {
    const chunks = await chunkMarkdown('');
    expect(chunks).toHaveLength(1);
    expect(chunks[0].text).toBe('');
  });

  it('should set correct line numbers', async () => {
    const lines = ['Line 0', 'Line 1', 'Line 2', 'Line 3', 'Line 4'];
    const content = lines.join('\n');
    const chunks = await chunkMarkdown(content);
    expect(chunks[0].startLine).toBe(0);
    expect(chunks[0].endLine).toBe(4);
  });

  it('should apply overlap between chunks', async () => {
    // Create content large enough to require splitting
    const lines = [];
    for (let i = 0; i < 100; i++) {
      lines.push(`Paragraph ${i}: ${'word '.repeat(30)}`);
    }
    const content = lines.join('\n');
    const chunks = await chunkMarkdown(content, { chunkTokenLimit: 128, chunkOverlap: 32 });

    if (chunks.length >= 2) {
      // The end of chunk 0 should overlap with the start of chunk 1
      const chunk0End = chunks[0].endLine;
      const chunk1Start = chunks[1].startLine;
      expect(chunk1Start).toBeLessThanOrEqual(chunk0End);
    }
  });
});
