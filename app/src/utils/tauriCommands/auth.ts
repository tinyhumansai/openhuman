/**
 * Authentication commands.
 */
import { invoke } from '@tauri-apps/api/core';
import { callCoreRpc } from '../../services/coreRpcClient';
import { isTauri, CommandResponse } from './common';

/**
 * Exchange a login token for a session token
 */
export async function exchangeToken(
  backendUrl: string,
  token: string
): Promise<{ sessionToken: string; user: object }> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }

  return await invoke('exchange_token', { backendUrl, token });
}

/**
 * Get the current authentication state from Rust
 */
export async function getAuthState(): Promise<{ is_authenticated: boolean; user: object | null }> {
  if (!isTauri()) {
    return { is_authenticated: false, user: null };
  }

  const response = await callCoreRpc<{ result: { isAuthenticated: boolean; user: object | null } }>(
    { method: 'openhuman.auth.get_state' }
  );

  return { is_authenticated: response.result.isAuthenticated, user: response.result.user };
}

/**
 * Get the session token from secure storage
 */
export async function getSessionToken(): Promise<string | null> {
  if (!isTauri()) {
    return null;
  }

  const response = await callCoreRpc<{ result: { token: string | null } }>({
    method: 'openhuman.auth.get_session_token',
  });
  return response.result.token;
}

/**
 * Logout and clear session
 */
export async function logout(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await callCoreRpc({ method: 'openhuman.auth.clear_session' });
}

/**
 * Store session in secure storage
 */
export async function storeSession(token: string, user: object): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await callCoreRpc({ method: 'openhuman.auth.store_session', params: { token, user } });
}

export async function openhumanEncryptSecret(plaintext: string): Promise<CommandResponse<string>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<string>>({
    method: 'openhuman.encrypt_secret',
    params: { plaintext },
  });
}

export async function openhumanDecryptSecret(ciphertext: string): Promise<CommandResponse<string>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<string>>({
    method: 'openhuman.decrypt_secret',
    params: { ciphertext },
  });
}
