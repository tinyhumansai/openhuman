import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockCallCoreRpc = vi.fn();

vi.mock('./coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
}));

describe('walletApi', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('fetchWalletStatus calls the wallet status RPC', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      result: {
        configured: true,
        onboardingCompleted: true,
        consentGranted: true,
        secretStored: true,
        source: 'generated',
        mnemonicWordCount: 12,
        accounts: [],
        updatedAtMs: 123,
      },
    });

    const { fetchWalletStatus } = await import('./walletApi');
    const result = await fetchWalletStatus();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.wallet_status' });
    expect(result.configured).toBe(true);
  });

  it('setupLocalWallet calls the wallet setup RPC with params', async () => {
    const payload = {
      consentGranted: true,
      source: 'imported' as const,
      mnemonicWordCount: 24,
      encryptedMnemonic: 'enc2:wallet-secret',
      accounts: [{ chain: 'evm' as const, address: '0xabc', derivationPath: "m/44'/60'/0'/0/0" }],
    };
    mockCallCoreRpc.mockResolvedValueOnce({ result: { configured: true } });

    const { setupLocalWallet } = await import('./walletApi');
    await setupLocalWallet(payload);

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.wallet_setup',
      params: payload,
    });
  });
});
