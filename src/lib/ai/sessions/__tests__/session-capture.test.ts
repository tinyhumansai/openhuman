import { describe, expect, it } from 'vitest';

import type { Message } from '../../providers/interface';
import { shouldCaptureSession } from '../session-capture';
import type { SessionEntry } from '../types';

function makeEntry(overrides: Partial<SessionEntry> = {}): SessionEntry {
  return {
    sessionId: 'test-session',
    updatedAt: Date.now(),
    sessionFile: 'test-session.jsonl',
    inputTokens: 1000,
    outputTokens: 500,
    totalTokens: 1500,
    model: 'test-model',
    compactionCount: 0,
    ...overrides,
  };
}

function makeUserMessage(text: string): Message {
  return { role: 'user', content: [{ type: 'text', text }] };
}

function makeAssistantMessage(text: string): Message {
  return { role: 'assistant', content: [{ type: 'text', text }] };
}

describe('shouldCaptureSession', () => {
  it('should return false for sessions with fewer than 2 user turns', () => {
    const messages: Message[] = [makeUserMessage('Hello'), makeAssistantMessage('Hi there!')];
    expect(shouldCaptureSession(messages, makeEntry())).toBe(false);
  });

  it('should return false for sessions with too few user tokens', () => {
    const messages: Message[] = [
      makeUserMessage('Hi'),
      makeAssistantMessage('Hello!'),
      makeUserMessage('Bye'),
      makeAssistantMessage('Goodbye!'),
    ];
    // "Hi" + "Bye" = 5 chars ~ 2 tokens, well below 100
    expect(shouldCaptureSession(messages, makeEntry())).toBe(false);
  });

  it('should return true for sessions with enough content', () => {
    // Need >= 100 estimated tokens from user content (~400 chars total)
    const longText1 =
      'Please help me understand how the memory system works in this application. I need to know about indexing, chunking, and search. Can you walk me through the full pipeline from file read to indexed chunks stored in SQLite?';
    const longText2 =
      'Can you explain the hybrid search algorithm in more detail? How does it combine FTS5 and vector similarity? What are the default weights and how does the scoring merge work? I want to understand the full ranking pipeline.';
    const messages: Message[] = [
      makeUserMessage(longText1),
      makeAssistantMessage('The memory system has three main components...'),
      makeUserMessage(longText2),
      makeAssistantMessage('Sure! The hybrid search works by...'),
    ];
    expect(shouldCaptureSession(messages, makeEntry())).toBe(true);
  });

  it('should return false if session was already flushed', () => {
    const longText = 'A'.repeat(500);
    const messages: Message[] = [
      makeUserMessage(longText),
      makeAssistantMessage('Response 1'),
      makeUserMessage(longText),
      makeAssistantMessage('Response 2'),
    ];
    const entry = makeEntry({
      compactionCount: 1,
      memoryFlushCompactionCount: 2, // Already flushed beyond current compaction
    });
    expect(shouldCaptureSession(messages, entry)).toBe(false);
  });

  it('should return true when flush count is less than compaction count', () => {
    const longText = 'A'.repeat(500);
    const messages: Message[] = [
      makeUserMessage(longText),
      makeAssistantMessage('Response 1'),
      makeUserMessage(longText),
      makeAssistantMessage('Response 2'),
    ];
    const entry = makeEntry({
      compactionCount: 2,
      memoryFlushCompactionCount: 1, // Flush is behind compaction
    });
    expect(shouldCaptureSession(messages, entry)).toBe(true);
  });

  it('should return true when no flush has occurred', () => {
    const longText = 'A'.repeat(500);
    const messages: Message[] = [
      makeUserMessage(longText),
      makeAssistantMessage('Response 1'),
      makeUserMessage(longText),
      makeAssistantMessage('Response 2'),
    ];
    const entry = makeEntry({
      compactionCount: 0,
      // memoryFlushCompactionCount is undefined
    });
    expect(shouldCaptureSession(messages, entry)).toBe(true);
  });

  it('should only count user messages for turn count', () => {
    const longText = 'A'.repeat(500);
    const messages: Message[] = [
      makeAssistantMessage('System message'),
      makeAssistantMessage('Another system message'),
      makeUserMessage(longText),
      makeAssistantMessage('Response'),
    ];
    // Only 1 user turn despite 4 total messages
    expect(shouldCaptureSession(messages, makeEntry())).toBe(false);
  });
});
