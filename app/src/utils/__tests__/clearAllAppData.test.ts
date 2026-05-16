import { beforeEach, describe, expect, it, vi } from 'vitest';

import { clearAllAppData } from '../clearAllAppData';

const { mockPurge, mockReset, mockRestart, mockPurgeCef } = vi.hoisted(() => ({
  mockPurge: vi.fn().mockResolvedValue(undefined),
  mockReset: vi.fn().mockResolvedValue(undefined),
  mockRestart: vi.fn().mockResolvedValue(undefined),
  mockPurgeCef: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('../../store', () => ({ persistor: { purge: mockPurge } }));

vi.mock('../tauriCommands', () => ({
  resetOpenHumanDataAndRestartCore: mockReset,
  restartApp: mockRestart,
  scheduleCefProfilePurge: mockPurgeCef,
}));

describe('clearAllAppData', () => {
  beforeEach(() => {
    mockPurge.mockReset().mockResolvedValue(undefined);
    mockReset.mockReset().mockResolvedValue(undefined);
    mockRestart.mockReset().mockResolvedValue(undefined);
    mockPurgeCef.mockReset().mockResolvedValue(undefined);
    window.localStorage.clear();
    window.sessionStorage.clear();
  });

  it('runs the full wipe sequence and restarts the app', async () => {
    const clearSession = vi.fn().mockResolvedValue(undefined);

    // Seed active-user key + user-1-scoped data + another user's data
    window.localStorage.setItem('OPENHUMAN_ACTIVE_USER_ID', 'user-1');
    window.localStorage.setItem('user-1:persist:accounts', 'a');
    window.localStorage.setItem('user-1:persist:coreMode', 'b');
    window.localStorage.setItem('user-2:persist:accounts', 'other');
    window.sessionStorage.setItem('session-persisted', '1');

    await clearAllAppData({ clearSession, userId: 'user-1' });

    expect(mockPurgeCef).toHaveBeenCalledWith('user-1');
    expect(clearSession).toHaveBeenCalledTimes(1);
    expect(mockReset).toHaveBeenCalledTimes(1);
    expect(mockPurge).toHaveBeenCalledTimes(1);
    // user-1's own keys are gone
    expect(window.localStorage.getItem('OPENHUMAN_ACTIVE_USER_ID')).toBeNull();
    expect(window.localStorage.getItem('user-1:persist:accounts')).toBeNull();
    expect(window.localStorage.getItem('user-1:persist:coreMode')).toBeNull();
    // user-2's keys are untouched (#983: other accounts must not lose data)
    expect(window.localStorage.getItem('user-2:persist:accounts')).toBe('other');
    expect(window.sessionStorage.getItem('session-persisted')).toBeNull();
    expect(mockRestart).toHaveBeenCalledTimes(1);
  });

  it('falls back to localStorage.clear() when no userId is provided', async () => {
    window.localStorage.setItem('user-1:persist:accounts', 'a');
    window.localStorage.setItem('user-2:persist:accounts', 'b');
    window.sessionStorage.setItem('session-persisted', '1');

    await clearAllAppData();

    expect(mockPurgeCef).toHaveBeenCalledWith(null);
    // No clearSession was provided — call sequence still completes.
    expect(mockReset).toHaveBeenCalledTimes(1);
    expect(mockRestart).toHaveBeenCalledTimes(1);
    // Without a userId we have no way to scope, so everything is cleared
    expect(window.localStorage.getItem('user-1:persist:accounts')).toBeNull();
    expect(window.localStorage.getItem('user-2:persist:accounts')).toBeNull();
  });

  it('continues if scheduleCefProfilePurge fails (best-effort)', async () => {
    mockPurgeCef.mockRejectedValueOnce(new Error('cef-purge boom'));

    await expect(clearAllAppData()).resolves.toBeUndefined();

    expect(mockReset).toHaveBeenCalledTimes(1);
    expect(mockRestart).toHaveBeenCalledTimes(1);
  });

  it('continues if clearSession fails (best-effort)', async () => {
    const clearSession = vi.fn().mockRejectedValue(new Error('logout boom'));

    await expect(clearAllAppData({ clearSession })).resolves.toBeUndefined();

    expect(clearSession).toHaveBeenCalledTimes(1);
    expect(mockReset).toHaveBeenCalledTimes(1);
    expect(mockRestart).toHaveBeenCalledTimes(1);
  });

  it('throws when resetOpenHumanDataAndRestartCore fails (unrecoverable)', async () => {
    mockReset.mockRejectedValueOnce(new Error('core reset boom'));

    await expect(clearAllAppData()).rejects.toThrow('core reset boom');

    expect(mockPurge).not.toHaveBeenCalled();
    expect(mockRestart).not.toHaveBeenCalled();
  });
});
