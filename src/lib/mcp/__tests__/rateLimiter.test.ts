import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  classifyTool,
  enforceRateLimit,
  isHeavyTool,
  isStateOnlyTool,
  RATE_LIMIT_CONFIG,
  resetRequestCallCount,
} from '../rateLimiter';

// Mock the logger module to avoid console output during tests
vi.mock('../logger', () => ({ mcpLog: vi.fn(), mcpWarn: vi.fn() }));

describe('RATE_LIMIT_CONFIG', () => {
  it('has correct configuration values', () => {
    expect(RATE_LIMIT_CONFIG.API_READ_DELAY_MS).toBe(500);
    expect(RATE_LIMIT_CONFIG.API_WRITE_DELAY_MS).toBe(1000);
    expect(RATE_LIMIT_CONFIG.MAX_CALLS_PER_MINUTE).toBe(30);
    expect(RATE_LIMIT_CONFIG.MAX_CALLS_PER_REQUEST).toBe(20);
  });
});

describe('classifyTool', () => {
  it('classifies known state_only tools correctly', () => {
    expect(classifyTool('get_chats')).toBe('state_only');
    expect(classifyTool('list_chats')).toBe('state_only');
    expect(classifyTool('get_chat')).toBe('state_only');
    expect(classifyTool('get_messages')).toBe('state_only');
    expect(classifyTool('list_messages')).toBe('state_only');
    expect(classifyTool('get_me')).toBe('state_only');
    expect(classifyTool('get_message_context')).toBe('state_only');
    expect(classifyTool('get_history')).toBe('state_only');
  });

  it('classifies known api_write tools correctly', () => {
    expect(classifyTool('send_message')).toBe('api_write');
    expect(classifyTool('edit_message')).toBe('api_write');
    expect(classifyTool('delete_message')).toBe('api_write');
    expect(classifyTool('forward_message')).toBe('api_write');
    expect(classifyTool('create_group')).toBe('api_write');
    expect(classifyTool('ban_user')).toBe('api_write');
  });

  it('classifies known api_read tools correctly', () => {
    expect(classifyTool('list_contacts')).toBe('api_read');
    expect(classifyTool('search_contacts')).toBe('api_read');
    expect(classifyTool('get_participants')).toBe('api_read');
    expect(classifyTool('get_admins')).toBe('api_read');
    expect(classifyTool('search_messages')).toBe('api_read');
  });

  it('defaults unknown tools to api_read', () => {
    expect(classifyTool('unknown_tool_xyz')).toBe('api_read');
    expect(classifyTool('random_tool_123')).toBe('api_read');
  });
});

describe('isStateOnlyTool', () => {
  it('returns true for state_only tools', () => {
    expect(isStateOnlyTool('get_chats')).toBe(true);
    expect(isStateOnlyTool('list_messages')).toBe(true);
    expect(isStateOnlyTool('get_me')).toBe(true);
  });

  it('returns false for api_write tools', () => {
    expect(isStateOnlyTool('send_message')).toBe(false);
    expect(isStateOnlyTool('edit_message')).toBe(false);
  });

  it('returns false for api_read tools', () => {
    expect(isStateOnlyTool('list_contacts')).toBe(false);
    expect(isStateOnlyTool('search_contacts')).toBe(false);
  });

  it('returns false for unknown tools', () => {
    expect(isStateOnlyTool('unknown_tool')).toBe(false);
  });
});

describe('isHeavyTool', () => {
  it('returns true for api_write tools', () => {
    expect(isHeavyTool('send_message')).toBe(true);
    expect(isHeavyTool('edit_message')).toBe(true);
    expect(isHeavyTool('delete_message')).toBe(true);
    expect(isHeavyTool('create_group')).toBe(true);
  });

  it('returns false for state_only tools', () => {
    expect(isHeavyTool('get_chats')).toBe(false);
    expect(isHeavyTool('list_messages')).toBe(false);
  });

  it('returns false for api_read tools', () => {
    expect(isHeavyTool('list_contacts')).toBe(false);
    expect(isHeavyTool('search_contacts')).toBe(false);
  });

  it('returns false for unknown tools', () => {
    expect(isHeavyTool('unknown_tool')).toBe(false);
  });
});

describe('enforceRateLimit', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    resetRequestCallCount();
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.useRealTimers();
  });

  it('allows state_only tools without any delay or limit', async () => {
    const start = Date.now();

    await enforceRateLimit('get_chats');
    await enforceRateLimit('list_messages');
    await enforceRateLimit('get_me');

    const elapsed = Date.now() - start;
    expect(elapsed).toBe(0); // No time should have passed
  });

  it('enforces per-request cap for api_read tools', async () => {
    // Call enforceRateLimit 20 times (the max)
    for (let i = 0; i < RATE_LIMIT_CONFIG.MAX_CALLS_PER_REQUEST; i++) {
      await enforceRateLimit('list_contacts');
      vi.advanceTimersByTime(RATE_LIMIT_CONFIG.API_READ_DELAY_MS);
    }

    // The 21st call should throw
    await expect(enforceRateLimit('list_contacts')).rejects.toThrow(
      /exceeded 20 API tool calls per request/
    );
  });

  it('enforces per-request cap for api_write tools', async () => {
    // Call enforceRateLimit 20 times (the max)
    for (let i = 0; i < RATE_LIMIT_CONFIG.MAX_CALLS_PER_REQUEST; i++) {
      const promise = enforceRateLimit('send_message');
      await vi.runAllTimersAsync();
      await promise;
    }

    // The 21st call should throw
    await expect(enforceRateLimit('send_message')).rejects.toThrow(
      /exceeded 20 API tool calls per request/
    );
  });

  it('does not enforce per-request cap on state_only tools', async () => {
    // Call state_only tools many more times than the cap
    for (let i = 0; i < 50; i++) {
      await enforceRateLimit('get_chats');
    }

    // Should not throw
    expect(true).toBe(true);
  });

  it('resets per-request counter when resetRequestCallCount is called', async () => {
    // Use up the budget
    for (let i = 0; i < RATE_LIMIT_CONFIG.MAX_CALLS_PER_REQUEST; i++) {
      const promise = enforceRateLimit('list_contacts');
      await vi.runAllTimersAsync();
      await promise;
    }

    // Next call should fail
    await expect(enforceRateLimit('list_contacts')).rejects.toThrow(
      /exceeded 20 API tool calls per request/
    );

    // Reset the counter
    resetRequestCallCount();

    // Now it should work again
    const promise = enforceRateLimit('list_contacts');
    await vi.runAllTimersAsync();
    await expect(promise).resolves.toBeUndefined();
  });

  it('enforces inter-call delay for api_read tools', async () => {
    const promise1 = enforceRateLimit('list_contacts');
    await vi.runAllTimersAsync();
    await promise1; // First call goes through immediately

    const start = Date.now();
    const promise2 = enforceRateLimit('list_contacts');

    // The second call should be delayed by API_READ_DELAY_MS
    expect(Date.now() - start).toBe(0); // Promise created but not resolved yet

    // Advance time and run all timers
    await vi.runAllTimersAsync();
    await promise2;

    expect(Date.now() - start).toBeGreaterThanOrEqual(RATE_LIMIT_CONFIG.API_READ_DELAY_MS);
  });

  it('enforces inter-call delay for api_write tools', async () => {
    const promise1 = enforceRateLimit('send_message');
    await vi.runAllTimersAsync();
    await promise1; // First call goes through immediately

    const start = Date.now();
    const promise2 = enforceRateLimit('send_message');

    // The second call should be delayed by API_WRITE_DELAY_MS
    expect(Date.now() - start).toBe(0);

    // Advance time and run all timers
    await vi.runAllTimersAsync();
    await promise2;

    expect(Date.now() - start).toBeGreaterThanOrEqual(RATE_LIMIT_CONFIG.API_WRITE_DELAY_MS);
  });

  it('enforces per-minute sliding window cap', async () => {
    // Make MAX_CALLS_PER_MINUTE calls within the window
    // Note: We need to reset the per-request counter periodically since MAX_CALLS_PER_MINUTE (30) > MAX_CALLS_PER_REQUEST (20)
    for (let i = 0; i < RATE_LIMIT_CONFIG.MAX_CALLS_PER_MINUTE; i++) {
      // Reset every 15 calls to avoid hitting per-request limit
      if (i > 0 && i % 15 === 0) {
        resetRequestCallCount();
      }
      const promise = enforceRateLimit('list_contacts');
      await vi.runAllTimersAsync();
      await promise;
    }

    // Reset again for the final test call
    resetRequestCallCount();

    // The next call should wait until the oldest entry expires (60 seconds from first call)
    // The implementation waits for: oldestTimestamp + 60_000 - now + 50
    const promise = enforceRateLimit('list_contacts');

    // Check that promise hasn't resolved yet
    let resolved = false;
    void promise.then(() => {
      resolved = true;
    });

    // Let microtasks run
    await Promise.resolve();
    expect(resolved).toBe(false);

    // Now advance time past the 60-second window and run timers
    await vi.runAllTimersAsync();

    // Promise should now resolve
    await promise;
    expect(resolved).toBe(true);
  });

  it('counts both api_read and api_write towards per-request cap', async () => {
    // Mix of read and write calls
    for (let i = 0; i < 10; i++) {
      const promise = enforceRateLimit('list_contacts');
      await vi.runAllTimersAsync();
      await promise;
    }

    for (let i = 0; i < 10; i++) {
      const promise = enforceRateLimit('send_message');
      await vi.runAllTimersAsync();
      await promise;
    }

    // Total is 20, so the next call should throw
    await expect(enforceRateLimit('list_contacts')).rejects.toThrow(
      /exceeded 20 API tool calls per request/
    );
  });
});
