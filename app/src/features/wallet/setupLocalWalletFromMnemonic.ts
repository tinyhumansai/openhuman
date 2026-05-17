import { setupLocalWallet } from '../../services/walletApi';
import {
  deriveAesKeyFromMnemonic,
  deriveWalletAccountsFromMnemonic,
  type WalletSetupSource,
} from '../../utils/cryptoKeys';
import { openhumanEncryptSecret } from '../../utils/tauriCommands/auth';

export async function persistLocalWalletFromMnemonic(args: {
  mnemonic: string;
  source: WalletSetupSource;
  setEncryptionKey: (value: string | null) => Promise<void>;
}): Promise<void> {
  const { mnemonic, source, setEncryptionKey } = args;
  const words = mnemonic.trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) {
    throw new Error('Recovery phrase is required.');
  }
  const normalizedMnemonic = words.join(' ');
  const aesKey = deriveAesKeyFromMnemonic(normalizedMnemonic);
  const encryptedMnemonic = (await openhumanEncryptSecret(normalizedMnemonic)).result?.trim();
  if (!encryptedMnemonic) {
    throw new Error('Failed to secure recovery phrase. Please try again.');
  }

  await setEncryptionKey(aesKey);
  await setupLocalWallet({
    consentGranted: true,
    source,
    mnemonicWordCount: words.length,
    encryptedMnemonic,
    accounts: deriveWalletAccountsFromMnemonic(normalizedMnemonic),
  });
}
