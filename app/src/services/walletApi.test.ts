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

  // fetchWalletBalances tests
  it('fetchWalletBalances calls wallet.balances via openhuman.wallet_balances and returns the array', async () => {
    const rows = [
      {
        chain: 'evm',
        evmNetwork: 'ethereum_mainnet',
        address: '0xABCD',
        assetSymbol: 'ETH',
        decimals: 18,
        raw: '1000000000000000000',
        formatted: '1.000000000000000000',
        providerStatus: 'ready',
      },
    ];
    mockCallCoreRpc.mockResolvedValueOnce({ result: rows });

    const { fetchWalletBalances } = await import('./walletApi');
    const result = await fetchWalletBalances();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.wallet_balances' });
    expect(result).toHaveLength(1);
    expect(result[0].assetSymbol).toBe('ETH');
    expect(result[0].providerStatus).toBe('ready');
  });

  it('fetchWalletBalances propagates RPC errors to the caller', async () => {
    mockCallCoreRpc.mockRejectedValueOnce(
      new Error('wallet is not configured; run wallet setup first')
    );

    const { fetchWalletBalances } = await import('./walletApi');
    await expect(fetchWalletBalances()).rejects.toThrow(
      'wallet is not configured; run wallet setup first'
    );
  });

  it('fetchWalletBalances maps an empty result array to an empty array', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: [] });

    const { fetchWalletBalances } = await import('./walletApi');
    const result = await fetchWalletBalances();

    expect(result).toEqual([]);
  });
});
