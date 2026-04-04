import { base64ToBytes, encryptIntegrationTokens } from '../../utils/integrationTokensCrypto';
import { callCoreCommand } from '../coreCommandClient';
import { callCoreRpc } from '../coreRpcClient';

interface IntegrationTokensResponse {
  success: boolean;
  data?: { encrypted: string };
}

interface IntegrationTokensPayload {
  accessToken: string;
  refreshToken?: string;
  expiresAt: string;
}

type LinkableChannel = 'telegram' | 'discord';

interface RawChannelLinkTokenData {
  token?: string;
  linkToken?: string;
  jwtToken?: string;
  url?: string;
  linkUrl?: string;
  authUrl?: string;
  deepLinkUrl?: string;
  expiresAt?: string;
  expires_at?: string;
  [key: string]: unknown;
}

export interface ChannelLinkTokenResult {
  token: string;
  launchUrl?: string;
  expiresAt?: string;
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

/**
 * Fetch encrypted OAuth tokens for an integration using a client-provided key.
 * POST /auth/integrations/:integrationId/tokens (auth required)
 */
export async function fetchIntegrationTokens(
  integrationId: string,
  key: string
): Promise<IntegrationTokensResponse> {
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

/**
 * Create a short-lived link token that can be handed to a messaging channel login flow.
 * POST /auth/channels/:channel/link-token (auth required)
 */
export async function createChannelLinkToken(
  channel: LinkableChannel
): Promise<ChannelLinkTokenResult> {
  const data = await callCoreCommand<RawChannelLinkTokenData>(
    'openhuman.auth_create_channel_link_token',
    { channel }
  );
  const token =
    typeof data?.token === 'string'
      ? data.token
      : typeof data?.linkToken === 'string'
        ? data.linkToken
        : typeof data?.jwtToken === 'string'
          ? data.jwtToken
          : '';

  if (!token) {
    throw new Error('Channel link token response missing token');
  }

  const launchUrlCandidates = [data?.url, data?.linkUrl, data?.authUrl, data?.deepLinkUrl];
  const launchUrl = launchUrlCandidates.find(
    (value): value is string => typeof value === 'string' && value.trim().length > 0
  );
  const expiresAt =
    typeof data?.expiresAt === 'string'
      ? data.expiresAt
      : typeof data?.expires_at === 'string'
        ? data.expires_at
        : undefined;

  return { token, launchUrl, expiresAt };
}
