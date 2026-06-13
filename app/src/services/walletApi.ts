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
  /** Present only when chain === 'evm'; identifies which EVM network the row is for. */
  evmNetwork?: EvmNetwork;
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
 *   account. The EVM account fans out into one row per displayed network
 *   (Ethereum, Base, BNB Chain); BTC/Solana/Tron return a single row each.
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

// ---------------------------------------------------------------------------
// Send / transfer surface
//
// The wallet uses a prepare-then-confirm-then-execute flow: `prepareTransfer`
// returns a quote (with the simulated fee) that must then be confirmed via
// `executePrepared`, which signs locally and broadcasts. Signing never leaves
// the core. Field names mirror the camelCase serde output in
// src/openhuman/wallet/execution.rs.
// ---------------------------------------------------------------------------

/** EVM network selector accepted by prepare_transfer / tx queries. */
export type EvmNetwork =
  | 'ethereum_mainnet'
  | 'base_mainnet'
  | 'arbitrum_one'
  | 'optimism_mainnet'
  | 'polygon_mainnet'
  | 'bsc_mainnet';

export type PreparedKind = 'native_transfer' | 'token_transfer';
export type PreparedStatus = 'awaiting_confirmation' | 'broadcasted' | 'consumed';

export interface PreparedTransaction {
  quoteId: string;
  kind: PreparedKind;
  chain: WalletChain;
  evmNetwork?: EvmNetwork;
  fromAddress: string;
  toAddress: string;
  assetSymbol: string;
  amountRaw: string;
  amountFormatted: string;
  estimatedFeeRaw: string;
  status: PreparedStatus;
  createdAtMs: number;
  expiresAtMs: number;
  notes: string[];
}

export interface ExecutionResult {
  quoteId: string;
  status: PreparedStatus;
  chain: WalletChain;
  evmNetwork?: EvmNetwork;
  transactionHash: string;
  explorerUrl?: string;
  transaction: PreparedTransaction;
}

export interface PrepareTransferParams {
  chain: WalletChain;
  toAddress: string;
  /** Amount in the asset's smallest unit (wei / sat / lamport / sun). */
  amountRaw: string;
  /** Omit / undefined for the chain's native asset. */
  assetSymbol?: string;
  /** Required only for chain === 'evm' to pick a network. */
  evmNetwork?: EvmNetwork;
}

/**
 * Build a transfer quote (simulated, not broadcast). Resolves to a
 * PreparedTransaction carrying the quoteId + estimated fee; rejection surfaces
 * a validation / provider error to show the user.
 */
export const prepareTransfer = async (
  params: PrepareTransferParams
): Promise<PreparedTransaction> => {
  const response = await callCoreRpc<{ result: PreparedTransaction }>({
    method: 'openhuman.wallet_prepare_transfer',
    params,
  });
  return response.result;
};

/**
 * Confirm and broadcast a previously prepared quote. `confirmed` is the
 * explicit safety boundary between simulate and execute.
 */
export const executePrepared = async (quoteId: string): Promise<ExecutionResult> => {
  const response = await callCoreRpc<{ result: ExecutionResult }>({
    method: 'openhuman.wallet_execute_prepared',
    params: { quoteId, confirmed: true },
  });
  return response.result;
};
