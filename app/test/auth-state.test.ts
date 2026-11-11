import { describe, expect, it } from 'vitest';

import { authStateValue } from './e2e/helpers/auth-state';

describe('authStateValue', () => {
  it('unwraps the logged Outcome result from auth_get_state', () => {
    const state = authStateValue({
      result: {
        result: { isAuthenticated: false, credential: undefined },
        logs: ['session state fetched'],
      },
    });

    expect(state.isAuthenticated).toBe(false);
  });

  it('accepts a bare auth DTO when the RPC operation has no log envelope', () => {
    const state = authStateValue({ result: { isAuthenticated: true, credential: 'session' } });

    expect(state.isAuthenticated).toBe(true);
    expect(state.credential).toBe('session');
  });

  it('rejects unrecognized RPC shapes instead of treating missing auth as signed out', () => {
    expect(() => authStateValue({ result: { logs: ['session state fetched'] } })).toThrow(
      'auth_get_state returned no boolean isAuthenticated field'
    );
  });
});
