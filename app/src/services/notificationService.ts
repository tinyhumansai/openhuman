import debug from 'debug';

import type { IntegrationNotification } from '../types/notifications';
import { callCoreRpc } from './coreRpcClient';

const log = debug('notifications');
const errLog = debug('notifications:error');

// ─────────────────────────────────────────────────────────────────────────────
// RPC wrappers
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Fetch paginated notifications from the core process.
 * Calls `openhuman.notification_list`.
 */
export async function fetchNotifications(opts?: {
  provider?: string;
  limit?: number;
  offset?: number;
  min_score?: number;
}): Promise<{ items: IntegrationNotification[]; unread_count: number }> {
  log('fetchNotifications %o', opts);
  const result = await callCoreRpc<{ items: IntegrationNotification[]; unread_count: number }>({
    method: 'openhuman.notification_list',
    params: opts ?? {},
  });
  log('fetchNotifications result: %d items, %d unread', result.items.length, result.unread_count);
  return result;
}

/**
 * Mark a single notification as read.
 * Calls `openhuman.notification_mark_read`.
 */
export async function markNotificationRead(id: string): Promise<void> {
  log('markNotificationRead id=%s', id);
  try {
    await callCoreRpc<{ ok: boolean }>({
      method: 'openhuman.notification_mark_read',
      params: { id },
    });
    log('markNotificationRead ok id=%s', id);
  } catch (err) {
    errLog('markNotificationRead failed id=%s: %o', id, err);
    throw err;
  }
}

export async function dismissNotification(id: string): Promise<void> {
  log('dismissNotification id=%s', id);
  try {
    await callCoreRpc<{ ok: boolean }>({
      method: 'openhuman.notification_dismiss',
      params: { id },
    });
    log('dismissNotification ok id=%s', id);
  } catch (err) {
    errLog('dismissNotification failed id=%s: %o', id, err);
    throw err;
  }
}

export async function markNotificationActed(id: string): Promise<void> {
  log('markNotificationActed id=%s', id);
  try {
    await callCoreRpc<{ ok: boolean }>({
      method: 'openhuman.notification_mark_acted',
      params: { id },
    });
    log('markNotificationActed ok id=%s', id);
  } catch (err) {
    errLog('markNotificationActed failed id=%s: %o', id, err);
    throw err;
  }
}
