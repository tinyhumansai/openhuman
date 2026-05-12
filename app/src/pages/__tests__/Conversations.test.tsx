import { describe, expect, it } from 'vitest';

import { formatThreadLoadError, isComposerInteractionBlocked } from '../Conversations';

describe('isComposerInteractionBlocked', () => {
  it('blocks composer interaction while the welcome agent loader is visible', () => {
    expect(
      isComposerInteractionBlocked({ activeThreadId: null, welcomePending: true, rustChat: true })
    ).toBe(true);
  });

  it('blocks composer interaction while a thread is actively running', () => {
    expect(
      isComposerInteractionBlocked({
        activeThreadId: 'thread-1',
        welcomePending: false,
        rustChat: true,
      })
    ).toBe(true);
  });

  it('allows composer interaction when chat is idle and ready', () => {
    expect(
      isComposerInteractionBlocked({ activeThreadId: null, welcomePending: false, rustChat: true })
    ).toBe(false);
  });

  it('blocks composer interaction when rust chat is unavailable', () => {
    expect(
      isComposerInteractionBlocked({ activeThreadId: null, welcomePending: false, rustChat: false })
    ).toBe(true);
  });
});

describe('formatThreadLoadError', () => {
  it('returns Error.message for native Error instances', () => {
    expect(formatThreadLoadError(new Error('boom'))).toBe('boom');
  });

  it('returns Redux SerializedError-shaped objects message field', () => {
    // createAsyncThunk re-throws { name, message, stack, code } from .unwrap()
    // when no rejectWithValue was used — that plain object is the original
    // Sentry report's payload.
    expect(
      formatThreadLoadError({
        name: 'Error',
        message: 'Core RPC openhuman.threads_list timed out after 30000ms',
        code: undefined,
      })
    ).toBe('Core RPC openhuman.threads_list timed out after 30000ms');
  });

  it('falls back to String(err) for objects with no message field', () => {
    expect(formatThreadLoadError({ foo: 'bar' })).toBe('[object Object]');
  });

  it('falls back to String(err) when err is a string', () => {
    expect(formatThreadLoadError('plain string')).toBe('plain string');
  });

  it('falls back to String(err) when err is null or undefined', () => {
    expect(formatThreadLoadError(null)).toBe('null');
    expect(formatThreadLoadError(undefined)).toBe('undefined');
  });

  it('ignores non-string message fields and falls back to String(err)', () => {
    expect(formatThreadLoadError({ message: 42 })).toBe('[object Object]');
  });
});
