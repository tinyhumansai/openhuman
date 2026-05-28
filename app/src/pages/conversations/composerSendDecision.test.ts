import { describe, expect, it } from 'vitest';

import {
  evaluateComposerSend,
  getComposerBlockedSendFeedback,
  handleComposerSlashCommand,
  shouldSendComposerKeyDown,
} from './composerSendDecision';

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

  it('blocks when no thread is selected', () => {
    const decision = evaluateComposerSend({
      rawText: 'hello',
      selectedThreadId: null,
      composerInteractionBlocked: false,
      isAtLimit: false,
      socketStatus: 'connected',
    });

    expect(decision.blockReason).toBe('missing_thread');
    expect(decision.shouldSend).toBe(false);
  });

  it('blocks while composer interaction is disabled', () => {
    const decision = evaluateComposerSend({
      rawText: 'hello',
      selectedThreadId: 'thread-1',
      composerInteractionBlocked: true,
      isAtLimit: false,
      socketStatus: 'connected',
    });

    expect(decision.blockReason).toBe('composer_blocked');
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
  it('consumes /new command', () => {
    expect(handleComposerSlashCommand('/new')).toEqual({ kind: 'new_or_clear' });
  });

  it('consumes /clear command (case-insensitive)', () => {
    expect(handleComposerSlashCommand('/CLEAR')).toEqual({ kind: 'new_or_clear' });
  });

  it('ignores normal chat text', () => {
    expect(handleComposerSlashCommand('hello')).toEqual({ kind: 'not_handled' });
  });
});

describe('shouldSendComposerKeyDown', () => {
  it('allows Enter to send when IME composition is inactive', () => {
    expect(shouldSendComposerKeyDown({ key: 'Enter' })).toBe(true);
  });

  it('does not send on Shift+Enter', () => {
    expect(shouldSendComposerKeyDown({ key: 'Enter', shiftKey: true })).toBe(false);
  });

  it('does not send while React reports IME composition', () => {
    expect(shouldSendComposerKeyDown({ key: 'Enter', nativeEvent: { isComposing: true } })).toBe(
      false
    );
  });

  it('does not send while the browser reports legacy IME keyCode 229', () => {
    expect(shouldSendComposerKeyDown({ key: 'Enter', nativeEvent: { keyCode: 229 } })).toBe(false);
  });

  it('does not send while textarea composition state is active', () => {
    expect(shouldSendComposerKeyDown({ key: 'Enter' }, true)).toBe(false);
  });
});

describe('getComposerBlockedSendFeedback', () => {
  it('returns error feedback for usage-limit blocking', () => {
    expect(getComposerBlockedSendFeedback('usage_limit_reached')).toEqual({
      error: {
        code: 'usage_limit_reached',
        message: 'Included budget exhausted. Top up credits or upgrade to continue.',
      },
    });
  });

  it('returns send error feedback for socket-disconnected blocking', () => {
    expect(getComposerBlockedSendFeedback('socket_disconnected')).toEqual({
      error: {
        code: 'socket_disconnected',
        message:
          'Realtime socket is not connected — responses cannot be delivered without a client ID.',
      },
    });
  });

  it('ignores block reasons that do not surface user feedback', () => {
    expect(getComposerBlockedSendFeedback('empty_input')).toBeNull();
    expect(getComposerBlockedSendFeedback(undefined)).toBeNull();
  });
});
