/** Auth state returned by `openhuman.auth_get_state`. */
export interface AuthStateResponse {
  isAuthenticated: boolean;
  userId?: string | null;
  credential?: 'session' | 'api-key' | 'local';
}

/** Read the auth DTO from the core RPC response. */
export function authStateValue(state: { result?: unknown }): AuthStateResponse {
  const raw = state.result;
  const envelope = raw && typeof raw === 'object' ? (raw as Record<string, unknown>) : {};
  const value =
    envelope.result && typeof envelope.result === 'object'
      ? (envelope.result as Record<string, unknown>)
      : envelope;

  if (typeof value.isAuthenticated !== 'boolean') {
    throw new Error('auth_get_state returned no boolean isAuthenticated field');
  }

  return value as unknown as AuthStateResponse;
}
