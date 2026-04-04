import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockCallCoreCommand = vi.fn();

vi.mock('../../coreCommandClient', () => ({
  callCoreCommand: (...args: unknown[]) => mockCallCoreCommand(...args),
}));

const { userApi } = await import('../userApi');

function getMockUser() {
  return {
    _id: 'user-123',
    telegramId: 12345678,
    hasAccess: true,
    magicWord: 'alpha',
    firstName: 'Test',
    lastName: 'User',
    username: 'testuser',
    role: 'user',
    activeTeamId: 'team-1',
    referral: {},
    subscription: { hasActiveSubscription: false, plan: 'FREE' },
    settings: {
      dailySummariesEnabled: false,
      dailySummaryChatIds: [],
      autoCompleteEnabled: false,
      autoCompleteVisibility: 'always',
      autoCompleteWhitelistChatIds: [],
      autoCompleteBlacklistChatIds: [],
    },
    usage: {
      cycleBudgetUsd: 10,
      remainingUsd: 10,
      spentThisCycleUsd: 0,
      spentTodayUsd: 0,
      cycleStartDate: new Date().toISOString(),
    },
    autoDeleteTelegramMessagesAfterDays: 30,
    autoDeleteThreadsAfterDays: 30,
  };
}

describe('userApi.getMe', () => {
  beforeEach(() => {
    mockCallCoreCommand.mockReset();
  });

  it('returns user data on success', async () => {
    mockCallCoreCommand.mockResolvedValue(getMockUser());

    const user = await userApi.getMe();

    expect(mockCallCoreCommand).toHaveBeenCalledWith('openhuman.auth_get_me');
    expect(user._id).toBe('user-123');
    expect(user.firstName).toBe('Test');
    expect(user.username).toBe('testuser');
    expect(user.subscription.plan).toBe('FREE');
  });

  it('throws when API returns error response', async () => {
    mockCallCoreCommand.mockRejectedValue(new Error('Unauthorized'));

    await expect(userApi.getMe()).rejects.toThrow();
  });

  it('throws when API returns success=false', async () => {
    mockCallCoreCommand.mockRejectedValue(new Error('Invalid token'));

    await expect(userApi.getMe()).rejects.toThrow('Invalid token');
  });

  it('throws on network error', async () => {
    mockCallCoreCommand.mockRejectedValue(new Error('Service unavailable'));

    await expect(userApi.getMe()).rejects.toBeDefined();
  });
});
