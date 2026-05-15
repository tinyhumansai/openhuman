/**
 * Authentication commands.
 */
import { invoke } from '@tauri-apps/api/core';

import { callCoreRpc } from '../../services/coreRpcClient';
import { CommandResponse, isTauri } from './common';

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
    { method: 'openhuman.auth_get_state' }
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
    method: 'openhuman.auth_get_session_token',
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

  await callCoreRpc({ method: 'openhuman.auth_clear_session' });
}

/**
 * Store session in secure storage
 */
export async function storeSession(token: string, user: object): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await callCoreRpc({ method: 'openhuman.auth_store_session', params: { token, user } });
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

/**
 * Summary of one stored provider credential profile. Mirrors the Rust
 * `credentials::responses::AuthProfileSummary` — no secret material is
 * carried over the wire; only existence + metadata.
 */
export interface AuthProfileSummary {
  id: string;
  provider: string;
  profile_name: string;
  kind: 'token' | 'oauth';
  account_id?: string | null;
  workspace_id?: string | null;
  has_token?: boolean;
  metadata?: Record<string, string>;
  created_at?: string;
  updated_at?: string;
}

/**
 * Store an API key for a cloud LLM provider (or any other named provider).
 * The token is encrypted at rest in `auth-profiles.json` under the workspace
 * `.secret_key` — same scheme used by the Composio integration.
 */
export async function authStoreProviderCredentials(args: {
  provider: string;
  profile?: string;
  token?: string;
  fields?: Record<string, string>;
  setActive?: boolean;
}): Promise<CommandResponse<AuthProfileSummary>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AuthProfileSummary>>({
    method: 'openhuman.auth_store_provider_credentials',
    params: args,
  });
}

/** Remove a stored provider credential profile. */
export async function authRemoveProviderCredentials(args: {
  provider: string;
  profile?: string;
}): Promise<CommandResponse<{ removed: boolean; provider: string; profile: string }>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<
    CommandResponse<{ removed: boolean; provider: string; profile: string }>
  >({ method: 'openhuman.auth_remove_provider_credentials', params: args });
}

/** List stored provider credential profiles, optionally filtered by provider. */
export async function authListProviderCredentials(
  provider?: string
): Promise<CommandResponse<AuthProfileSummary[]>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AuthProfileSummary[]>>({
    method: 'openhuman.auth_list_provider_credentials',
    params: provider ? { provider } : {},
  });
}
