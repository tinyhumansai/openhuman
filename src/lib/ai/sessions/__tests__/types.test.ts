import { describe, expect, it } from 'vitest';

import { DEFAULT_SESSION_CONFIG, type SessionConfig, type SessionEntry } from '../types';

describe('DEFAULT_SESSION_CONFIG', () => {
  it('should have reasonable default values', () => {
    expect(DEFAULT_SESSION_CONFIG.maxContextTokens).toBe(100000);
    expect(DEFAULT_SESSION_CONFIG.preserveRecentTokens).toBe(20000);
    expect(DEFAULT_SESSION_CONFIG.memoryFlushEnabled).toBe(true);
  });

  it('should have preserveRecentTokens less than maxContextTokens', () => {
    expect(DEFAULT_SESSION_CONFIG.preserveRecentTokens).toBeLessThan(
      DEFAULT_SESSION_CONFIG.maxContextTokens
    );
  });
});

describe('SessionEntry type', () => {
  it('should accept a valid session entry object', () => {
    const entry: SessionEntry = {
      sessionId: 'test-123',
      updatedAt: Date.now(),
      sessionFile: 'test-123.jsonl',
      inputTokens: 1000,
      outputTokens: 500,
      totalTokens: 1500,
      model: 'custom-model',
      compactionCount: 0,
    };
    expect(entry.sessionId).toBe('test-123');
    expect(entry.model).toBe('custom-model');
  });

  it('should accept optional fields', () => {
    const entry: SessionEntry = {
      sessionId: 'test-456',
      updatedAt: Date.now(),
      sessionFile: 'test-456.jsonl',
      inputTokens: 0,
      outputTokens: 0,
      totalTokens: 0,
      model: 'test',
      compactionCount: 2,
      memoryFlushAt: Date.now(),
      memoryFlushCompactionCount: 1,
      label: 'Test Session',
      channel: 'telegram',
    };
    expect(entry.label).toBe('Test Session');
    expect(entry.channel).toBe('telegram');
    expect(entry.memoryFlushCompactionCount).toBe(1);
  });
});

describe('SessionConfig type', () => {
  it('should accept partial config merged with defaults', () => {
    const partial: Partial<SessionConfig> = { maxContextTokens: 50000 };
    const merged = { ...DEFAULT_SESSION_CONFIG, ...partial };
    expect(merged.maxContextTokens).toBe(50000);
    expect(merged.preserveRecentTokens).toBe(20000);
    expect(merged.memoryFlushEnabled).toBe(true);
  });
});
