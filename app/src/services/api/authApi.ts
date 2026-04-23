import { getBackendUrl } from '../backendUrl';
import { callCoreRpc } from '../coreRpcClient';

const EMAIL_MAGIC_LINK_TIMEOUT_MS = 15_000;

/**
 * Send a magic-link email for email-based login.
 * POST /auth/email/send-link
 * @param email - The user's email address.
 * @param frontendRedirectUri - Where the backend should redirect after verification
 *   (e.g. "openhuman://" for desktop, or the web app origin for web).
 */
export async function sendEmailMagicLink(
  email: string,
  frontendRedirectUri: string,
  timeoutMs = EMAIL_MAGIC_LINK_TIMEOUT_MS
): Promise<void> {
  const backendUrl = await getBackendUrl();
  const controller = new AbortController();
  const timeoutId = window.setTimeout(() => controller.abort(), timeoutMs);

  try {
    const response = await fetch(`${backendUrl}/auth/email/send-link`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ email, frontendRedirectUri }),
      signal: controller.signal,
    });
    if (!response.ok) {
      const body = (await response.json().catch(() => ({}))) as { error?: string };
      throw new Error(body.error ?? `Failed to send magic link (${response.status})`);
    }
  } catch (error) {
    if (
      (error instanceof DOMException && error.name === 'AbortError') ||
      (error instanceof Error && error.name === 'AbortError')
    ) {
      throw new Error('Request timed out. Please try again.');
    }
    throw error;
  } finally {
    window.clearTimeout(timeoutId);
  }
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
