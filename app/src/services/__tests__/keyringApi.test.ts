import { afterEach, describe, expect, it, vi } from 'vitest';

import { decideKeyringConsent, fetchKeyringStatus, retryKeyringProbe } from '../keyringApi';

vi.mock('../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const { callCoreRpc } = await import('../coreRpcClient');
const mockRpc = vi.mocked(callCoreRpc);

describe('keyringApi', () => {
  afterEach(() => vi.clearAllMocks());

  it('fetchKeyringStatus calls correct RPC method', async () => {
    const status = { available: true, activeMode: 'os_keyring', backendName: 'os' };
    mockRpc.mockResolvedValueOnce({ result: status });

    const result = await fetchKeyringStatus();
    expect(result).toEqual(status);
    expect(mockRpc).toHaveBeenCalledWith({ method: 'openhuman.keyring_consent_status' });
  });

  it('decideKeyringConsent sends mode parameter', async () => {
    const pref = { storageMode: 'local_encrypted', consentedAtMs: 123 };
    mockRpc.mockResolvedValueOnce({ result: pref });

    const result = await decideKeyringConsent('local_encrypted');
    expect(result).toEqual(pref);
    expect(mockRpc).toHaveBeenCalledWith({
      method: 'openhuman.keyring_consent_decide',
      params: { mode: 'local_encrypted' },
    });
  });

  it('retryKeyringProbe calls correct RPC method', async () => {
    const status = { available: false, activeMode: 'consent_pending', backendName: 'os' };
    mockRpc.mockResolvedValueOnce({ result: status });

    const result = await retryKeyringProbe();
    expect(result).toEqual(status);
    expect(mockRpc).toHaveBeenCalledWith({ method: 'openhuman.keyring_consent_retry_probe' });
  });
});
