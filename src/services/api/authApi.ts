import { apiClient } from '../apiClient';

interface ConsumeLoginTokenResponse {
  success: boolean;
  data: { jwtToken: string };
}

/**
 * Consume a verified login token and return the JWT.
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
