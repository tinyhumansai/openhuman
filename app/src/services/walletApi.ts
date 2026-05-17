import { callCoreRpc } from './coreRpcClient';

export type WalletChain = 'evm' | 'btc' | 'solana' | 'tron';
export type WalletSetupSource = 'generated' | 'imported';

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
