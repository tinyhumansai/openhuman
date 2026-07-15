import { describe, expect, it } from 'vitest';

import {
  formatThreadLoadError,
  isComposerInteractionBlocked,
  isImeCompositionKeyEvent,
  sortAgentProfiles,
} from '../../features/conversations/Conversations';
import type { AgentProfile } from '../../types/agentProfile';

describe('isComposerInteractionBlocked', () => {
  it('blocks composer interaction while the selected thread is actively running', () => {
    expect(isComposerInteractionBlocked({ selectedThreadActive: true, rustChat: true })).toBe(true);
  });

  it('allows composer interaction when the selected thread is idle and ready', () => {
    expect(isComposerInteractionBlocked({ selectedThreadActive: false, rustChat: true })).toBe(
      false
    );
  });

  it('blocks composer interaction when rust chat is unavailable', () => {
    expect(isComposerInteractionBlocked({ selectedThreadActive: false, rustChat: false })).toBe(
      true
    );
  });
});

describe('isImeCompositionKeyEvent', () => {
  it('detects active IME composition from the native event', () => {
    expect(isImeCompositionKeyEvent({ nativeEvent: { isComposing: true } })).toBe(true);
  });

  it('detects legacy IME keyCode 229 fallbacks', () => {
    expect(isImeCompositionKeyEvent({ keyCode: 229 })).toBe(true);
    expect(isImeCompositionKeyEvent({ which: 229 })).toBe(true);
    expect(isImeCompositionKeyEvent({ nativeEvent: { keyCode: 229 } })).toBe(true);
    expect(isImeCompositionKeyEvent({ nativeEvent: { which: 229 } })).toBe(true);
  });

  it('does not treat ordinary Enter as IME composition', () => {
    expect(isImeCompositionKeyEvent({ keyCode: 13, nativeEvent: { isComposing: false } })).toBe(
      false
    );
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

describe('sortAgentProfiles', () => {
  it('filters out built-in profiles', () => {
    const profiles: AgentProfile[] = [
      { id: '1', name: 'Custom', builtIn: false, description: '', agentId: 'orchestrator' },
      { id: 'default', name: 'Default', builtIn: true, description: '', agentId: 'orchestrator' },
    ];
    expect(sortAgentProfiles(profiles, 'en')).toHaveLength(1);
  });

  it('sorts by sortOrder then name', () => {
    const profiles: AgentProfile[] = [
      {
        id: 'a',
        name: 'Alpha',
        builtIn: false,
        sortOrder: 2,
        description: '',
        agentId: 'orchestrator',
      },
      {
        id: 'b',
        name: 'Beta',
        builtIn: false,
        sortOrder: 1,
        description: '',
        agentId: 'orchestrator',
      },
      {
        id: 'c',
        name: 'Charlie',
        builtIn: false,
        sortOrder: 1,
        description: '',
        agentId: 'orchestrator',
      },
    ];
    const sorted = sortAgentProfiles(profiles, 'en');
    expect(sorted[0].id).toBe('b');
    expect(sorted[1].id).toBe('c');
    expect(sorted[2].id).toBe('a');
  });

  it('treats missing sortOrder as 0', () => {
    const profiles: AgentProfile[] = [
      {
        id: 'a',
        name: 'Alpha',
        builtIn: false,
        sortOrder: null,
        description: '',
        agentId: 'orchestrator',
      },
      {
        id: 'b',
        name: 'Beta',
        builtIn: false,
        sortOrder: 1,
        description: '',
        agentId: 'orchestrator',
      },
    ];
    const sorted = sortAgentProfiles(profiles, 'en');
    expect(sorted[0].id).toBe('a');
    expect(sorted[1].id).toBe('b');
  });

  it('returns empty array when all profiles are built-in', () => {
    expect(
      sortAgentProfiles(
        [
          {
            id: 'default',
            name: 'Default',
            builtIn: true,
            description: '',
            agentId: 'orchestrator',
          },
        ],
        'en'
      )
    ).toHaveLength(0);
  });

  it('returns empty array for empty input', () => {
    expect(sortAgentProfiles([], 'en')).toHaveLength(0);
  });
});
