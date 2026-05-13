import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../../../services/coreRpcClient';
import { openhumanGetClientConfig } from '../config';

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

  it('dispatches openhuman.config_get_client_config and returns the response', async () => {
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

    expect(callCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.config_get_client_config' });
    expect(got).toEqual(expected);
  });
});
