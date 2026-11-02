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

/** Resolve one exact TinySecurity keyring operation waiting for host approval. */
export const respondToKeyringOperation = async (
  requestId: string,
  approved: boolean
): Promise<void> => {
  await callCoreRpc({
    method: 'openhuman.keyring_consent_respond_operation',
    params: { request_id: requestId, approved },
  });
};
