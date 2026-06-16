import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../../../services/coreRpcClient';
import {
  openhumanClaudeCodeAuthStatus,
  openhumanClaudeCodeSetFullAccess,
  openhumanClaudeCodeSettings,
  openhumanGetClientConfig,
} from '../config';

vi.mock('../../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

vi.mock('../common', () => ({ isTauri: vi.fn(() => true), CommandResponse: undefined }));

describe('openhumanGetClientConfig', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.resetAllMocks();
  });

  it('throws when not running inside the Tauri shell', async () => {
    const { isTauri } = await import('../common');
    vi.mocked(isTauri).mockReturnValueOnce(false);
    await expect(openhumanGetClientConfig()).rejects.toThrow(/Not running in Tauri/i);
  });

  it('dispatches openhuman.inference_get_client_config and returns the response', async () => {
    const expected = {
      result: {
        api_url: 'https://api.openai.com/v1/chat/completions',
        default_model: 'gpt-4o',
        app_version: '0.0.0-test',
        api_key_set: true,
      },
      messages: [],
    };
    vi.mocked(callCoreRpc).mockResolvedValueOnce(expected);

    const got = await openhumanGetClientConfig();

    expect(callCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.inference_get_client_config' });
    expect(got).toEqual(expected);
  });
});

describe('Claude Code wrappers', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });
  afterEach(() => {
    vi.resetAllMocks();
  });

  it('openhumanClaudeCodeAuthStatus dispatches the bare auth-status RPC', async () => {
    const auth = { source: 'subscription', account_email: 'a@b.co', last_checked: 1 };
    vi.mocked(callCoreRpc).mockResolvedValueOnce(auth as never);
    const got = await openhumanClaudeCodeAuthStatus();
    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.inference_claude_code_auth_status',
    });
    expect(got).toEqual(auth);
  });

  it('openhumanClaudeCodeSettings dispatches the bare settings RPC', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ full_access: true } as never);
    const got = await openhumanClaudeCodeSettings();
    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.inference_claude_code_settings',
    });
    expect(got).toEqual({ full_access: true });
  });

  it('openhumanClaudeCodeSetFullAccess passes the enabled flag as params', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ full_access: false } as never);
    const got = await openhumanClaudeCodeSetFullAccess(false);
    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.inference_claude_code_set_full_access',
      params: { enabled: false },
    });
    expect(got).toEqual({ full_access: false });
  });

  it.each([
    ['openhumanClaudeCodeAuthStatus', () => openhumanClaudeCodeAuthStatus()],
    ['openhumanClaudeCodeSettings', () => openhumanClaudeCodeSettings()],
    ['openhumanClaudeCodeSetFullAccess', () => openhumanClaudeCodeSetFullAccess(true)],
  ])('%s throws outside the Tauri shell', async (_name, call) => {
    const { isTauri } = await import('../common');
    vi.mocked(isTauri).mockReturnValueOnce(false);
    await expect(call()).rejects.toThrow(/Not running in Tauri/i);
  });
});
