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

  it.each([
    'gmail',
    'slack',
    'whatsapp',
    'wechat',
    'telegram',
    'discord',
    'linkedin',
    'outlook',
    'instagram',
    'twitter',
  ])('routes %s provider to /chat', provider => {
    expect(resolveIntegrationRoute(makeIntegration({ provider }))).toBe('/chat');
  });

  it('falls back to the notifications main view for unknown providers', () => {
    expect(resolveIntegrationRoute(makeIntegration({ provider: 'unknown-app' }))).toBe(
      '/notifications?view=main'
    );
  });

  it('prefers deep_link over provider default', () => {
    const n = makeIntegration({ provider: 'slack', deep_link: '/skills' });
    expect(resolveIntegrationRoute(n)).toBe('/skills');
  });

  it.each([
    'javascript:alert(1)',
    '//evil.example.com',
    'https://evil.example.com/x',
    'http://evil.example.com',
    'data:text/html,x',
    'mailto:x@y.z',
    'relative/no/leading/slash',
    '\\evil',
  ])('ignores unsafe deep_link %s and uses the provider default', deep_link => {
    // `deep_link` derives from untrusted inbound provider content, so a value
    // that is not a relative in-app path must be dropped, not navigated to.
    const n = makeIntegration({ provider: 'slack', deep_link });
    expect(resolveIntegrationRoute(n)).toBe('/chat');
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

  it('routes skills category to /connections (Phase 2 rename)', () => {
    expect(resolveSystemRoute(makeSystem({ category: 'skills' }))).toBe('/connections');
  });

  it('routes system category to /home', () => {
    expect(resolveSystemRoute(makeSystem({ category: 'system' }))).toBe('/home');
  });

  it('routes meetings category to the notifications main view', () => {
    expect(resolveSystemRoute(makeSystem({ category: 'meetings' }))).toBe(
      '/notifications?view=main'
    );
  });

  it('routes reminders category to the notifications main view', () => {
    expect(resolveSystemRoute(makeSystem({ category: 'reminders' }))).toBe(
      '/notifications?view=main'
    );
  });

  it('routes important category to the notifications main view', () => {
    expect(resolveSystemRoute(makeSystem({ category: 'important' }))).toBe(
      '/notifications?view=main'
    );
  });

  it('prefers deepLink over category default', () => {
    const item = makeSystem({ category: 'messages', deepLink: '/notifications' });
    expect(resolveSystemRoute(item)).toBe('/notifications');
  });

  it.each([
    'javascript:alert(1)',
    '//evil.example.com',
    'https://evil.example.com/x',
    'data:text/html,x',
    'relative/no/leading/slash',
    '\\evil',
  ])('ignores unsafe deepLink %s and uses the category default', deepLink => {
    const item = makeSystem({ category: 'messages', deepLink });
    expect(resolveSystemRoute(item)).toBe('/chat');
  });
});
