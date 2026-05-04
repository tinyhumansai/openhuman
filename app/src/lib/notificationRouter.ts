import debug from 'debug';

import type { NotificationItem } from '../store/notificationSlice';
import type { IntegrationNotification } from '../types/notifications';

const log = debug('notifications:router');

// ─────────────────────────────────────────────────────────────────────────────
// Known in-app hash routes
// ─────────────────────────────────────────────────────────────────────────────

const ROUTES = {
  chat: '/chat',
  skills: '/skills',
  home: '/home',
  notifications: '/notifications',
} as const;

/**
 * Providers whose notifications belong in the unified chat / accounts view.
 * Add new provider slugs here as integrations are added.
 */
const MESSAGE_PROVIDERS = new Set([
  'gmail',
  'slack',
  'whatsapp',
  'telegram',
  'discord',
  'linkedin',
]);

// ─────────────────────────────────────────────────────────────────────────────
// Route resolvers
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Resolve a hash-router path for an integration (provider) notification.
 *
 * Priority:
 *   1. Explicit `deep_link` set by the core triage pipeline.
 *   2. Provider default — message providers → /chat.
 *   3. `/notifications` fallback.
 */
export function resolveIntegrationRoute(n: IntegrationNotification): string {
  if (n.deep_link) {
    log('[notification-router] integration id=%s explicit deep_link=%s', n.id, n.deep_link);
    return n.deep_link;
  }

  if (MESSAGE_PROVIDERS.has(n.provider)) {
    log('[notification-router] integration id=%s provider=%s → /chat', n.id, n.provider);
    return ROUTES.chat;
  }

  log(
    '[notification-router] integration id=%s provider=%s → /notifications (fallback)',
    n.id,
    n.provider
  );
  return ROUTES.notifications;
}

/**
 * Resolve a hash-router path for a system-event (`NotificationItem`) notification.
 *
 * Priority:
 *   1. Explicit `deepLink` stored on the item.
 *   2. Category default: messages/agents → /chat; skills → /skills; system → /home.
 *   3. `/notifications` fallback.
 */
export function resolveSystemRoute(item: NotificationItem): string {
  if (item.deepLink) {
    log('[notification-router] system id=%s explicit deepLink=%s', item.id, item.deepLink);
    return item.deepLink;
  }

  switch (item.category) {
    case 'messages':
      log('[notification-router] system id=%s category=messages → /chat', item.id);
      return ROUTES.chat;
    case 'agents':
      log('[notification-router] system id=%s category=agents → /chat', item.id);
      return ROUTES.chat;
    case 'skills':
      log('[notification-router] system id=%s category=skills → /skills', item.id);
      return ROUTES.skills;
    case 'system':
      log('[notification-router] system id=%s category=system → /home', item.id);
      return ROUTES.home;
    default:
      log(
        '[notification-router] system id=%s category=%s → /notifications (fallback)',
        item.id,
        item.category
      );
      return ROUTES.notifications;
  }
}
