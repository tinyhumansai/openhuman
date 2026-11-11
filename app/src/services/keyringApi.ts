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

/**
 * Keyring operations waiting for approval. The renderer polls this when it has
 * no socket yet (before sign-in), so the approval prompt still reaches it.
 */
export const listPendingKeyringOperations = async (): Promise<unknown[]> => {
  // The controller answers with the bare array; accept a `{ result }` envelope too.
  const response = await callCoreRpc<unknown>({
    method: 'openhuman.keyring_consent_list_pending_operations',
  });
  if (Array.isArray(response)) return response;
  const wrapped = (response as { result?: unknown } | null)?.result;
  return Array.isArray(wrapped) ? wrapped : [];
};
