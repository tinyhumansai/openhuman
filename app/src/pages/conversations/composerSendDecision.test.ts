import { describe, expect, it } from 'vitest';

import { evaluateComposerSend, handleComposerSlashCommand } from './composerSendDecision';

describe('evaluateComposerSend', () => {
  it('blocks empty input', () => {
    const decision = evaluateComposerSend({
      rawText: '   ',
      selectedThreadId: 'thread-1',
      composerInteractionBlocked: false,
      isAtLimit: false,
      socketStatus: 'connected',
    });

    expect(decision).toEqual({ shouldSend: false, trimmedText: '', blockReason: 'empty_input' });
  });

  it('blocks usage limit', () => {
    const decision = evaluateComposerSend({
      rawText: 'hello',
      selectedThreadId: 'thread-1',
      composerInteractionBlocked: false,
      isAtLimit: true,
      socketStatus: 'connected',
    });

    expect(decision.blockReason).toBe('usage_limit_reached');
    expect(decision.shouldSend).toBe(false);
  });

  it('blocks when socket is disconnected', () => {
    const decision = evaluateComposerSend({
      rawText: 'hello',
      selectedThreadId: 'thread-1',
      composerInteractionBlocked: false,
      isAtLimit: false,
      socketStatus: 'disconnected',
    });

    expect(decision.blockReason).toBe('socket_disconnected');
    expect(decision.shouldSend).toBe(false);
  });

  it('allows send path setup for valid chat send input', () => {
    const decision = evaluateComposerSend({
      rawText: ' hello ',
      selectedThreadId: 'thread-1',
      composerInteractionBlocked: false,
      isAtLimit: false,
      socketStatus: 'connected',
    });

    expect(decision).toEqual({ shouldSend: true, trimmedText: 'hello' });
  });
});

describe('handleComposerSlashCommand', () => {
  it('consumes /new and blocks thread reset when welcome lock is active', () => {
    expect(handleComposerSlashCommand('/new', true)).toEqual({
      kind: 'new_or_clear',
      blockedByWelcomeLock: true,
    });
  });
});
