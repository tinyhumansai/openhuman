import {
  type LoopbackHandle,
  startLoopbackOauthListener,
  type StartLoopbackOptions,
} from './loopbackOauthListener';
import { openUrl } from './openUrl';

const OPENROUTER_AUTH_URL = 'https://openrouter.ai/auth';
const OPENROUTER_TOKEN_URL = 'https://openrouter.ai/api/v1/auth/keys';
const PKCE_METHOD = 'S256';
const OPENROUTER_LOOPBACK_PORT = 3000;
const VERIFIER_ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~';

interface OpenRouterExchangeResponse {
  key?: string;
  error?: { message?: string } | string;
}

export interface OpenRouterOAuthDeps {
  startLoopbackListener?: (options?: StartLoopbackOptions) => Promise<LoopbackHandle | null>;
  openExternalUrl?: (url: string) => Promise<void>;
  fetchImpl?: typeof fetch;
  signal?: AbortSignal;
}

function randomVerifier(length = 64): string {
  const bytes = new Uint8Array(length);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, value => VERIFIER_ALPHABET[value % VERIFIER_ALPHABET.length]).join('');
}

function base64UrlEncode(bytes: Uint8Array): string {
  let binary = '';
  for (const value of bytes) {
    binary += String.fromCharCode(value);
  }
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/g, '');
}

async function createCodeChallenge(verifier: string): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(verifier));
  return base64UrlEncode(new Uint8Array(digest));
}

function extractOAuthCode(callbackUrl: string, expectedState: string): string {
  let parsed: URL;
  try {
    parsed = new URL(callbackUrl);
  } catch {
    throw new Error('OpenRouter OAuth returned an invalid callback URL.');
  }

  const actualState = parsed.searchParams.get('state');
  if (actualState !== expectedState) {
    throw new Error('OpenRouter OAuth callback state did not match the request.');
  }

  const code = parsed.searchParams.get('code');
  if (!code) {
    throw new Error('OpenRouter OAuth did not return an authorization code.');
  }
  return code;
}

async function exchangeCodeForKey(
  code: string,
  verifier: string,
  fetchImpl: typeof fetch
): Promise<string> {
  const response = await fetchImpl(OPENROUTER_TOKEN_URL, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ code, code_verifier: verifier, code_challenge_method: PKCE_METHOD }),
  });

  let body: OpenRouterExchangeResponse | null = null;
  try {
    body = (await response.json()) as OpenRouterExchangeResponse;
  } catch {
    body = null;
  }

  if (!response.ok) {
    const detail =
      typeof body?.error === 'string'
        ? body.error
        : body?.error && typeof body.error === 'object'
          ? body.error.message
          : null;
    throw new Error(detail || `OpenRouter key exchange failed (${response.status}).`);
  }

  if (!body?.key || typeof body.key !== 'string') {
    throw new Error('OpenRouter key exchange succeeded but no API key was returned.');
  }

  return body.key;
}

function toOpenRouterCallbackUrl(redirectUri: string): string {
  let parsed: URL;
  try {
    parsed = new URL(redirectUri);
  } catch {
    throw new Error('OpenRouter OAuth listener returned an invalid redirect URL.');
  }

  // Preserve the port the loopback listener actually bound to (carried in
  // redirectUri): when the requested port is busy, the Tauri command falls back
  // to an OS-assigned ephemeral port, so hardcoding OPENROUTER_LOOPBACK_PORT here
  // sent OpenRouter a callback_url pointing at the wrong port. The PKCE
  // callback_url is per-request, so the dynamic port is valid (this matches the
  // sibling OAuthProviderButton flow, which trusts the bound port).
  parsed.hostname = 'localhost';
  return parsed.toString();
}

export async function connectOpenRouterViaOAuth(deps: OpenRouterOAuthDeps = {}): Promise<string> {
  const startLoopbackListener = deps.startLoopbackListener ?? startLoopbackOauthListener;
  const openExternalUrl = deps.openExternalUrl ?? openUrl;
  const fetchImpl = deps.fetchImpl ?? fetch;
  const signal = deps.signal;

  const loopback = await startLoopbackListener({ port: OPENROUTER_LOOPBACK_PORT });
  if (!loopback) {
    throw new Error('OpenRouter OAuth requires the desktop app. Use an API key instead.');
  }

  if (signal?.aborted) {
    await loopback.cancel();
    throw new Error('OpenRouter OAuth was cancelled.');
  }

  const verifier = randomVerifier();
  const challenge = await createCodeChallenge(verifier);
  const authUrl = new URL(OPENROUTER_AUTH_URL);
  authUrl.searchParams.set('callback_url', toOpenRouterCallbackUrl(loopback.redirectUri));
  authUrl.searchParams.set('code_challenge', challenge);
  authUrl.searchParams.set('code_challenge_method', PKCE_METHOD);

  try {
    await openExternalUrl(authUrl.toString());
    const callbackUrl = await Promise.race([
      loopback.awaitCallback(),
      new Promise<string>((_, reject) => {
        if (!signal) return;
        const onAbort = () => {
          signal.removeEventListener('abort', onAbort);
          reject(new Error('OpenRouter OAuth was cancelled.'));
        };
        signal.addEventListener('abort', onAbort, { once: true });
      }),
    ]);
    const code = extractOAuthCode(callbackUrl, loopback.state);
    return await exchangeCodeForKey(code, verifier, fetchImpl);
  } finally {
    await loopback.cancel();
  }
}
