import { describe, expect, it } from 'vitest';

import type { NotificationItem } from '../store/notificationSlice';
import type { IntegrationNotification } from '../types/notifications';
import { resolveIntegrationRoute, resolveSystemRoute } from './notificationRouter';

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

const makeIntegration = (
  overrides: Partial<IntegrationNotification> = {}
): IntegrationNotification => ({
  id: 'i-1',
  provider: 'slack',
  title: 'Test',
  body: 'Body',
  raw_payload: {},
  status: 'unread',
  received_at: '2026-04-29T00:00:00Z',
  ...overrides,
});

const makeSystem = (overrides: Partial<NotificationItem> = {}): NotificationItem => ({
  id: 's-1',
  category: 'messages',
  title: 'Test',
  body: 'Body',
  timestamp: 1,
  read: false,
  ...overrides,
});

// ─────────────────────────────────────────────────────────────────────────────
// resolveIntegrationRoute
// ─────────────────────────────────────────────────────────────────────────────

describe('resolveIntegrationRoute', () => {
  it('returns explicit deep_link when present', () => {
    const n = makeIntegration({ deep_link: '/chat?account=abc' });
    expect(resolveIntegrationRoute(n)).toBe('/chat?account=abc');
  });

  it.each(['gmail', 'slack', 'whatsapp', 'telegram', 'discord', 'linkedin'])(
    'routes %s provider to /chat',
    provider => {
      expect(resolveIntegrationRoute(makeIntegration({ provider }))).toBe('/chat');
    }
  );

  it('falls back to /notifications for unknown providers', () => {
    expect(resolveIntegrationRoute(makeIntegration({ provider: 'unknown-app' }))).toBe(
      '/notifications'
    );
  });

  it('prefers deep_link over provider default', () => {
    const n = makeIntegration({ provider: 'slack', deep_link: '/skills' });
    expect(resolveIntegrationRoute(n)).toBe('/skills');
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// resolveSystemRoute
// ─────────────────────────────────────────────────────────────────────────────

describe('resolveSystemRoute', () => {
  it('returns explicit deepLink when present', () => {
    const item = makeSystem({ deepLink: '/skills' });
    expect(resolveSystemRoute(item)).toBe('/skills');
  });

  it('routes messages category to /chat', () => {
    expect(resolveSystemRoute(makeSystem({ category: 'messages' }))).toBe('/chat');
  });

  it('routes agents category to /chat', () => {
    expect(resolveSystemRoute(makeSystem({ category: 'agents' }))).toBe('/chat');
  });

  it('routes skills category to /skills', () => {
    expect(resolveSystemRoute(makeSystem({ category: 'skills' }))).toBe('/skills');
  });

  it('routes system category to /home', () => {
    expect(resolveSystemRoute(makeSystem({ category: 'system' }))).toBe('/home');
  });

  it('prefers deepLink over category default', () => {
    const item = makeSystem({ category: 'messages', deepLink: '/notifications' });
    expect(resolveSystemRoute(item)).toBe('/notifications');
  });
});
