import { callCoreRpc } from '../coreRpcClient';

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
