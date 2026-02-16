import { http, HttpResponse } from 'msw';
import { describe, expect, it, vi } from 'vitest';

import { server } from '../../../test/server';

// Mock the store import that apiClient depends on
vi.mock('../../../store', () => ({
  store: { getState: () => ({ auth: { token: 'test-jwt-token' } }) },
}));

// Mock the config to use test backend URL
vi.mock('../../../utils/config', () => ({ BACKEND_URL: 'http://localhost:5005', IS_DEV: true }));

// Import after mocks
const { userApi } = await import('../userApi');

describe('userApi.getMe', () => {
  it('returns user data on success', async () => {
    // Default handler from handlers.ts already handles this
    const user = await userApi.getMe();
    expect(user._id).toBe('user-123');
    expect(user.firstName).toBe('Test');
    expect(user.username).toBe('testuser');
    expect(user.subscription.plan).toBe('FREE');
  });

  it('throws when API returns error response', async () => {
    server.use(
      http.get('http://localhost:5005/telegram/me', () => {
        return HttpResponse.json({ success: false, error: 'Unauthorized' }, { status: 401 });
      })
    );

    await expect(userApi.getMe()).rejects.toThrow();
  });

  it('throws when API returns success=false', async () => {
    server.use(
      http.get('http://localhost:5005/telegram/me', () => {
        return HttpResponse.json({ success: false, error: 'Invalid token' });
      })
    );

    await expect(userApi.getMe()).rejects.toThrow('Invalid token');
  });

  it('throws on network error', async () => {
    server.use(
      http.get('http://localhost:5005/telegram/me', () => {
        return HttpResponse.error();
      })
    );

    await expect(userApi.getMe()).rejects.toBeDefined();
  });
});
