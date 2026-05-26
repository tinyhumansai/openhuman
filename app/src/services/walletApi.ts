import { callCoreRpc } from './coreRpcClient';

export type WalletChain = 'evm' | 'btc' | 'solana' | 'tron';
export type WalletSetupSource = 'generated' | 'imported';

/**
 * A single balance row returned by wallet.balances.
 * Field names match the camelCase serde output of BalanceInfo in
 * src/openhuman/wallet/execution.rs.
 */
export interface BalanceInfo {
  chain: WalletChain;
  /** Present only when chain === 'evm'. */
  evmNetwork?: string;
  address: string;
  assetSymbol: string;
  decimals: number;
  /** Raw balance in the chain's smallest unit (wei / sat / lamport / sun). */
  raw: string;
  /** Human-readable formatted balance (e.g. "1.234"). */
  formatted: string;
  /** "ready" when the RPC provider responded; "missing" when it fell back to zero. */
  providerStatus: 'ready' | 'missing';
}

export interface WalletAccount {
  chain: WalletChain;
  address: string;
  derivationPath: string;
}

export interface WalletStatus {
  configured: boolean;
  onboardingCompleted: boolean;
  consentGranted: boolean;
  secretStored: boolean;
  source: WalletSetupSource | null;
  mnemonicWordCount: number | null;
  accounts: WalletAccount[];
  updatedAtMs: number | null;
}

export interface SetupWalletParams {
  consentGranted: boolean;
  source: WalletSetupSource;
  mnemonicWordCount: number;
  encryptedMnemonic?: string;
  accounts: WalletAccount[];
}

export const fetchWalletStatus = async (): Promise<WalletStatus> => {
  const response = await callCoreRpc<{ result: WalletStatus }>({
    method: 'openhuman.wallet_status',
  });
  return response.result;
};

export const setupLocalWallet = async (params: SetupWalletParams): Promise<WalletStatus> => {
  const response = await callCoreRpc<{ result: WalletStatus }>({
    method: 'openhuman.wallet_setup',
    params,
  });
  return response.result;
};

/**
 * Fetch native-asset balances for every derived wallet account.
 *
 * Calls `wallet.balances` via the core RPC relay. The contract:
 * - When the wallet IS configured, the core returns one row per derived
 *   account (EVM/BTC/Solana/Tron) and this resolves to that array.
 * - When the wallet IS NOT configured (no recovery phrase set up yet), the
 *   core returns an RPC error; this promise rejects so callers can surface
 *   the empty / setup-required state rather than silently rendering nothing.
 */
export const fetchWalletBalances = async (): Promise<BalanceInfo[]> => {
  const response = await callCoreRpc<{ result: BalanceInfo[] }>({
    method: 'openhuman.wallet_balances',
  });
  return response.result;
};
