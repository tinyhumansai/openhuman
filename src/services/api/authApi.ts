import { apiClient } from '../apiClient';

interface ConsumeLoginTokenResponse {
  success: boolean;
  data: { jwtToken: string };
}

interface IntegrationTokensResponse {
  success: boolean;
  data?: { encrypted: string };
}

/**
 * Consume a verified login token and return the JWT.
 * Works for both Telegram and OAuth login tokens.
 * POST /telegram/login-tokens/:token/consume (no auth required)
 */
export async function consumeLoginToken(loginToken: string): Promise<string> {
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
  return apiClient.post<IntegrationTokensResponse>(
    `/auth/integrations/${encodeURIComponent(integrationId)}/tokens`,
    { key }
  );
}
