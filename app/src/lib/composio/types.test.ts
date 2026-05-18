import { describe, expect, it } from 'vitest';

import { type ComposioConnection, deriveComposioState } from './types';

function connection(status: string): ComposioConnection {
  return { id: `ca_${status.toLowerCase()}`, toolkit: 'gmail', status };
}

describe('deriveComposioState', () => {
  it('treats expired Composio auth as a first-class expired state', () => {
    expect(deriveComposioState(connection('EXPIRED'))).toBe('expired');
  });

  it('keeps failed and generic error statuses as error', () => {
    expect(deriveComposioState(connection('FAILED'))).toBe('error');
    expect(deriveComposioState(connection('ERROR'))).toBe('error');
  });

  it('keeps active and pending statuses unchanged', () => {
    expect(deriveComposioState(connection('ACTIVE'))).toBe('connected');
    expect(deriveComposioState(connection('CONNECTED'))).toBe('connected');
    expect(deriveComposioState(connection('PENDING'))).toBe('pending');
  });
});
