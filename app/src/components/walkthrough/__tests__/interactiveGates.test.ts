import { beforeEach, describe, expect, it, vi } from 'vitest';

// Import the mock store for state manipulation
import { store } from '../../../store';
import { GATE_CONNECT_SKILL, GATE_SEND_MESSAGE, getStepGate } from '../interactiveGates';

// Mock the store module
vi.mock('../../../store', () => {
  let mockState = { accounts: { order: [] }, thread: { messagesByThreadId: {} } };
  return {
    store: {
      getState: vi.fn(() => mockState),
      dispatch: vi.fn(),
      // Expose a setter so tests can update the state
      __setMockState: (s: typeof mockState) => {
        mockState = s;
      },
    },
  };
});

const setMockState = (store as any).__setMockState;

beforeEach(() => {
  setMockState({ accounts: { order: [] }, thread: { messagesByThreadId: {} } });
});

describe('GATE_CONNECT_SKILL', () => {
  it('returns false when no accounts connected', () => {
    expect(GATE_CONNECT_SKILL.isComplete()).toBe(false);
  });

  it('returns true when at least one account is connected', () => {
    setMockState({ accounts: { order: ['acct-1'] }, thread: { messagesByThreadId: {} } });
    expect(GATE_CONNECT_SKILL.isComplete()).toBe(true);
  });
});

describe('GATE_SEND_MESSAGE', () => {
  it('returns false when no messages exist', () => {
    expect(GATE_SEND_MESSAGE.isComplete()).toBe(false);
  });

  it('returns false when only agent messages exist', () => {
    setMockState({
      accounts: { order: [] },
      thread: { messagesByThreadId: { t1: [{ id: 'm1', sender: 'agent', content: 'Hello' }] } },
    });
    expect(GATE_SEND_MESSAGE.isComplete()).toBe(false);
  });

  it('returns true when a user message exists', () => {
    setMockState({
      accounts: { order: [] },
      thread: { messagesByThreadId: { t1: [{ id: 'm1', sender: 'user', content: 'Hi there' }] } },
    });
    expect(GATE_SEND_MESSAGE.isComplete()).toBe(true);
  });

  it('returns true when user message exists among agent messages', () => {
    setMockState({
      accounts: { order: [] },
      thread: {
        messagesByThreadId: {
          t1: [
            { id: 'm1', sender: 'agent', content: 'Hello' },
            { id: 'm2', sender: 'user', content: 'Hi' },
          ],
        },
      },
    });
    expect(GATE_SEND_MESSAGE.isComplete()).toBe(true);
  });
});

describe('getStepGate', () => {
  it('returns null when step has no data', () => {
    const step = { target: 'body', content: 'test' } as any;
    expect(getStepGate(step)).toBeNull();
  });

  it('returns null when step data has no gateId', () => {
    const step = { target: 'body', content: 'test', data: {} } as any;
    expect(getStepGate(step)).toBeNull();
  });

  it('returns null for unknown gateId', () => {
    const step = { target: 'body', content: 'test', data: { gateId: 'nonexistent' } } as any;
    expect(getStepGate(step)).toBeNull();
  });

  it('returns GATE_CONNECT_SKILL for matching gateId', () => {
    const step = { target: 'body', content: 'test', data: { gateId: 'connect-skill' } } as any;
    expect(getStepGate(step)).toBe(GATE_CONNECT_SKILL);
  });

  it('returns GATE_SEND_MESSAGE for matching gateId', () => {
    const step = { target: 'body', content: 'test', data: { gateId: 'send-message' } } as any;
    expect(getStepGate(step)).toBe(GATE_SEND_MESSAGE);
  });
});
