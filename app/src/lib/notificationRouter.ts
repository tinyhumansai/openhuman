import debug from 'debug';

import type { NotificationItem } from '../store/notificationSlice';
import type { IntegrationNotification } from '../types/notifications';

const log = debug('notifications:router');

// ─────────────────────────────────────────────────────────────────────────────
// Known in-app hash routes
// ─────────────────────────────────────────────────────────────────────────────

const ROUTES = {
  chat: '/chat',
  skills: '/connections',
  home: '/home',
  notifications: '/notifications?view=main',
} as const;

/**
 * Providers whose notifications belong in the unified chat / accounts view.
 * Add new provider slugs here as integrations are added.
 */
const MESSAGE_PROVIDERS = new Set([
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
]);

/**
 * A `deep_link` / `deepLink` is set by the core triage pipeline, which derives
 * it from untrusted inbound provider content. Callers feed the result straight
 * to react-router `navigate()`, so accept it only when it is a relative in-app
 * path: a single leading `/`, no protocol-relative `//`, no scheme, no
 * backslash. This blocks `javascript:` / `http(s)://evil` / `//host` from
 * reaching the router (HashRouter already contains most of these, but the
 * allowlist keeps navigation strictly in-app). Unsafe values fall through to
 * the provider/category default.
 */
function isSafeInAppPath(path: string): boolean {
  // A single leading slash, no protocol-relative `//`, no backslash. Keeps the
  // value a relative in-app route, so `javascript:` / `http(s)://evil` / `//host`
  // can never reach react-router `navigate()`.
  return path.startsWith('/') && path[1] !== '/' && path[1] !== '\\' && !path.includes('\\');
}

// ─────────────────────────────────────────────────────────────────────────────
// Route resolvers
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Resolve a hash-router path for an integration (provider) notification.
 *
 * Priority:
 *   1. Explicit `deep_link` set by the core triage pipeline.
 *   2. Provider default — message providers → /chat.
 *   3. `/notifications?view=main` fallback.
 */
export function resolveIntegrationRoute(n: IntegrationNotification): string {
  if (n.deep_link && isSafeInAppPath(n.deep_link)) {
    log('[notification-router] integration id=%s explicit deep_link=%s', n.id, n.deep_link);
    return n.deep_link;
  }
  if (n.deep_link) {
    log('[notification-router] integration id=%s ignored unsafe deep_link=%s', n.id, n.deep_link);
  }

  if (MESSAGE_PROVIDERS.has(n.provider)) {
    log('[notification-router] integration id=%s provider=%s → /chat', n.id, n.provider);
    return ROUTES.chat;
  }

  log(
    '[notification-router] integration id=%s provider=%s → /notifications?view=main (fallback)',
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
 *   3. `/notifications?view=main` fallback.
 */
export function resolveSystemRoute(item: NotificationItem): string {
  if (item.deepLink && isSafeInAppPath(item.deepLink)) {
    log('[notification-router] system id=%s explicit deepLink=%s', item.id, item.deepLink);
    return item.deepLink;
  }
  if (item.deepLink) {
    log('[notification-router] system id=%s ignored unsafe deepLink=%s', item.id, item.deepLink);
  }

  switch (item.category) {
    case 'messages':
      log('[notification-router] system id=%s category=messages → /chat', item.id);
      return ROUTES.chat;
    case 'agents':
      log('[notification-router] system id=%s category=agents → /chat', item.id);
      return ROUTES.chat;
    case 'skills':
      log('[notification-router] system id=%s category=skills → /connections', item.id);
      return ROUTES.skills;
    case 'system':
      log('[notification-router] system id=%s category=system → /home', item.id);
      return ROUTES.home;
    case 'meetings':
      log(
        '[notification-router] system id=%s category=meetings → /notifications?view=main',
        item.id
      );
      return ROUTES.notifications;
    case 'reminders':
      log(
        '[notification-router] system id=%s category=reminders → /notifications?view=main',
        item.id
      );
      return ROUTES.notifications;
    case 'important':
      log(
        '[notification-router] system id=%s category=important → /notifications?view=main',
        item.id
      );
      return ROUTES.notifications;
    default:
      log(
        '[notification-router] system id=%s category=%s → /notifications?view=main (fallback)',
        item.id,
        item.category
      );
      return ROUTES.notifications;
  }
}
