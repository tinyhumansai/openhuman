import { describe, expect, it, vi } from 'vitest';

import { resolveHomeUserName } from '../Home';

const mockUseUser = vi.fn(() => ({ user: { firstName: 'Shrey' } }));

vi.mock('../../components/ConnectionIndicator', () => ({
  default: () => <div>Connection Indicator</div>,
}));

vi.mock('../../hooks/useUser', () => ({ useUser: () => mockUseUser() }));

vi.mock('../../utils/config', async importOriginal => {
  const actual = await importOriginal<typeof import('../../utils/config')>();
  return { ...actual, APP_VERSION: '0.0.0-test' };
});

describe('resolveHomeUserName', () => {
  it('uses camelCase name fields when present', () => {
    expect(resolveHomeUserName({ firstName: 'Ada', lastName: 'Lovelace' })).toBe('Ada Lovelace');
  });

  it('falls back to snake_case name fields from core snapshot payloads', () => {
    expect(resolveHomeUserName({ first_name: 'Ada', last_name: 'Lovelace' })).toBe('Ada Lovelace');
  });

  it('falls back to username when no name fields are present', () => {
    expect(resolveHomeUserName({ username: 'openhuman' })).toBe('@openhuman');
  });

  it('falls back to the email local-part when no explicit name exists', () => {
    expect(resolveHomeUserName({ email: 'ada@example.com' })).toBe('ada');
  });
});
