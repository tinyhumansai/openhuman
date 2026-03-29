import { isTauri as coreIsTauri } from '@tauri-apps/api/core';

import { base64ToBytes, encryptIntegrationTokens } from '../../utils/integrationTokensCrypto';
import { apiClient } from '../apiClient';
import { callCoreRpc } from '../coreRpcClient';

interface ConsumeLoginTokenResponse {
  success: boolean;
  data: { jwtToken: string };
}

interface IntegrationTokensResponse {
  success: boolean;
  data?: { encrypted: string };
}

interface IntegrationTokensPayload {
  accessToken: string;
  refreshToken?: string;
  expiresAt: string;
}

function bytesToHex(bytes: Uint8Array): string {
  return Array.from(bytes)
    .map(byte => byte.toString(16).padStart(2, '0'))
    .join('');
}

function normalizeKeyToHex(rawKey: string): string {
  const trimmed = rawKey.trim();
  const maybeHex = trimmed.replace(/^0x/i, '');
  if (maybeHex.length === 64 && /^[0-9a-fA-F]+$/.test(maybeHex)) {
    return maybeHex.toLowerCase();
  }
  return bytesToHex(base64ToBytes(trimmed));
}

/**
 * Consume a verified login token and return the JWT.
 * Works for both Telegram and OAuth login tokens.
 * POST /telegram/login-tokens/:token/consume (no auth required)
 */
export async function consumeLoginToken(loginToken: string): Promise<string> {
  if (coreIsTauri()) {
    const response = await callCoreRpc<{ result: { jwtToken: string } }>({
      method: 'openhuman.auth.consume_login_token',
      params: { loginToken },
    });
    const jwtToken = response.result?.jwtToken;
    if (!jwtToken) {
      throw new Error('Login token invalid or expired');
    }
    return jwtToken;
  }

  const response = await apiClient.post<ConsumeLoginTokenResponse>(
    `/telegram/login-tokens/${encodeURIComponent(loginToken)}/consume`,
    undefined,
    { requireAuth: false }
  );
  console.log('[ConsumeLoginToken] Response', response);
  if (!response.success || !response.data?.jwtToken) {
    throw new Error('Login token invalid or expired');
  }
  return response.data.jwtToken;
}

/**
 * Fetch encrypted OAuth tokens for an integration using a client-provided key.
 * POST /auth/integrations/:integrationId/tokens (auth required)
 */
export async function fetchIntegrationTokens(
  integrationId: string,
  key: string
): Promise<IntegrationTokensResponse> {
  if (coreIsTauri()) {
    const response = await callCoreRpc<{ result: IntegrationTokensPayload }>({
      method: 'openhuman.auth.oauth_fetch_integration_tokens',
      params: { integrationId, key },
    });
    const tokens = response.result;
    if (!tokens?.accessToken || !tokens?.expiresAt) {
      throw new Error('Integration token handoff did not return required fields');
    }

    const encrypted = await encryptIntegrationTokens(
      JSON.stringify({
        accessToken: tokens.accessToken,
        refreshToken: tokens.refreshToken ?? '',
        expiresAt: tokens.expiresAt,
      }),
      normalizeKeyToHex(key)
    );
    return { success: true, data: { encrypted } };
  }

  return apiClient.post<IntegrationTokensResponse>(
    `/auth/integrations/${encodeURIComponent(integrationId)}/tokens`,
    { key }
  );
}
