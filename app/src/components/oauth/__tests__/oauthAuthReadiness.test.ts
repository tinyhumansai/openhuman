import { beforeEach, describe, expect, it, vi } from 'vitest';

import { getCoreStateSnapshot } from '../../../lib/coreState/store';
import { bootCheckTransport } from '../../../services/bootCheckService';
import { testCoreRpcConnection } from '../../../services/coreRpcClient';
import { isTauri } from '../../../services/webviewAccountService';
import {
  getStoredCoreMode,
  getStoredCoreToken,
  storeCoreMode,
} from '../../../utils/configPersistence';
import {
  oauthAuthReadinessUserMessage,
  prepareOAuthLoginLaunch,
  waitForOAuthAuthReadiness,
} from '../oauthAuthReadiness';

vi.mock('../../../lib/coreState/store', () => ({ getCoreStateSnapshot: vi.fn() }));

vi.mock('../../../services/coreRpcClient', () => ({
  getCoreRpcUrl: vi.fn().mockResolvedValue('http://127.0.0.1:7788/rpc'),
  testCoreRpcConnection: vi.fn(),
}));

vi.mock('../../../services/bootCheckService', () => ({
  bootCheckTransport: { invokeCmd: vi.fn().mockResolvedValue(undefined), callRpc: vi.fn() },
}));

vi.mock('../../../utils/configPersistence', () => ({
  getStoredCoreMode: vi.fn(),
  getStoredCoreToken: vi.fn().mockReturnValue(null),
  storeCoreMode: vi.fn(),
}));

vi.mock('../../../services/webviewAccountService', () => ({
  isTauri: vi.fn().mockReturnValue(true),
}));

describe('oauthAuthReadiness', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(getStoredCoreMode).mockReturnValue('local');
    vi.mocked(getCoreStateSnapshot).mockReturnValue({
      isBootstrapping: false,
      isReady: true,
      snapshot: {
        sessionToken: null,
        auth: { isAuthenticated: false, userId: null, user: null, profileId: null },
        currentUser: null,
        onboardingCompleted: false,
        chatOnboardingCompleted: false,
        analyticsEnabled: false,
        meetAutoOrchestratorHandoff: false,
        localState: { encryptionKey: null, onboardingTasks: null },
        runtime: { screenIntelligence: null, localAi: null, autocomplete: null, service: null },
      },
      teams: [],
      teamMembersById: {},
      teamInvitesById: {},
    });
    vi.mocked(testCoreRpcConnection).mockResolvedValue({ ok: true } as Response);
    vi.mocked(isTauri).mockReturnValue(true);
  });

  it('defaults to local mode in Tauri when no core mode is stored', async () => {
    vi.mocked(getStoredCoreMode).mockReturnValue(null);
    // isTauri is true from beforeEach — should auto-set 'local' and proceed

    const result = await waitForOAuthAuthReadiness(2_000);

    expect(vi.mocked(storeCoreMode)).toHaveBeenCalledWith('local');
    expect(result).toEqual({ ready: true });
  });

  it('returns core_mode_unset when BootCheckGate has not committed a mode', async () => {
    vi.mocked(getStoredCoreMode).mockReturnValue(null);
    // Must be web (non-Tauri) — in Tauri the code defaults to 'local' and never
    // returns core_mode_unset.
    vi.mocked(isTauri).mockReturnValue(false);

    const result = await waitForOAuthAuthReadiness(500);

    expect(result).toEqual({ ready: false, reason: 'core_mode_unset' });
    expect(oauthAuthReadinessUserMessage('core_mode_unset')).toMatch(/setup screen/i);
  });

  it('returns ready when core mode and ping are satisfied', async () => {
    const result = await waitForOAuthAuthReadiness(2_000);

    expect(result).toEqual({ ready: true });
    expect(bootCheckTransport.invokeCmd).toHaveBeenCalledWith('start_core_process', {});
    expect(testCoreRpcConnection).toHaveBeenCalled();
  });

  it('returns core_unreachable when ping never succeeds', async () => {
    vi.mocked(testCoreRpcConnection).mockResolvedValue({ ok: false } as Response);

    const result = await waitForOAuthAuthReadiness(600);

    expect(result).toEqual({ ready: false, reason: 'core_unreachable' });
  });

  it('does not block first-login callbacks on CoreStateProvider bootstrap once ping succeeds', async () => {
    vi.mocked(getCoreStateSnapshot).mockReturnValue({
      isBootstrapping: true,
      isReady: false,
      snapshot: {
        sessionToken: null,
        auth: { isAuthenticated: false, userId: null, user: null, profileId: null },
        currentUser: null,
        onboardingCompleted: false,
        chatOnboardingCompleted: false,
        analyticsEnabled: false,
        meetAutoOrchestratorHandoff: false,
        localState: { encryptionKey: null, onboardingTasks: null },
        runtime: { screenIntelligence: null, localAi: null, autocomplete: null, service: null },
      },
      teams: [],
      teamMembersById: {},
      teamInvitesById: {},
    });

    const result = await waitForOAuthAuthReadiness(3_000);

    expect(result).toEqual({ ready: true });
  });

  it('does not start the local core on web builds', async () => {
    vi.mocked(isTauri).mockReturnValue(false);

    await waitForOAuthAuthReadiness(1_000);

    expect(bootCheckTransport.invokeCmd).not.toHaveBeenCalled();
  });

  it('starts the local core only once during pre-launch readiness', async () => {
    await prepareOAuthLoginLaunch();

    expect(bootCheckTransport.invokeCmd).toHaveBeenCalledTimes(1);
    expect(bootCheckTransport.invokeCmd).toHaveBeenCalledWith('start_core_process', {});
  });

  it('rejects with the readiness message when pre-launch core readiness fails', async () => {
    vi.useFakeTimers();
    vi.mocked(testCoreRpcConnection).mockResolvedValue({ ok: false } as Response);

    try {
      const launch = expect(prepareOAuthLoginLaunch()).rejects.toThrow(
        oauthAuthReadinessUserMessage('core_unreachable')
      );

      await vi.advanceTimersByTimeAsync(8_000);

      await launch;
    } finally {
      vi.useRealTimers();
    }
  });

  it('returns cloud-specific message for core_unreachable when mode is cloud', () => {
    vi.mocked(getStoredCoreMode).mockReturnValue('cloud');
    const msg = oauthAuthReadinessUserMessage('core_unreachable');
    expect(msg).toMatch(/remote.*cloud/i);
    expect(msg).toMatch(/RPC URL/i);
  });

  it('returns local-specific message for core_unreachable when mode is local', () => {
    vi.mocked(getStoredCoreMode).mockReturnValue('local');
    const msg = oauthAuthReadinessUserMessage('core_unreachable');
    expect(msg).toMatch(/local runtime/i);
    expect(msg).toMatch(/Quit and reopen/i);
  });

  it('passes cloud token to testCoreRpcConnection when mode is cloud', async () => {
    vi.mocked(getStoredCoreMode).mockReturnValue('cloud');
    vi.mocked(getStoredCoreToken).mockReturnValue('cloud-bearer-token');

    await waitForOAuthAuthReadiness(2_000);

    expect(testCoreRpcConnection).toHaveBeenCalledWith(
      'http://127.0.0.1:7788/rpc',
      'cloud-bearer-token'
    );
  });
});
