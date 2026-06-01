import { callCoreRpc } from './coreRpcClient';
import type { KeyringConsentPreference, KeyringStatus } from './coreStateApi';

export const fetchKeyringStatus = async (): Promise<KeyringStatus> => {
  const response = await callCoreRpc<{ result: KeyringStatus }>({
    method: 'openhuman.keyring_consent_status',
  });
  return response.result;
};

export const decideKeyringConsent = async (
  mode: 'local_encrypted' | 'declined'
): Promise<KeyringConsentPreference> => {
  const response = await callCoreRpc<{ result: KeyringConsentPreference }>({
    method: 'openhuman.keyring_consent_decide',
    params: { mode },
  });
  return response.result;
};

export const retryKeyringProbe = async (): Promise<KeyringStatus> => {
  const response = await callCoreRpc<{ result: KeyringStatus }>({
    method: 'openhuman.keyring_consent_retry_probe',
  });
  return response.result;
};
