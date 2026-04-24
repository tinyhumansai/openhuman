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

type NotificationIngestResult = { id: string; skipped?: false } | { skipped: true; reason: string };
type NotificationStats = {
  total: number;
  unread: number;
  unscored: number;
  by_provider: Record<string, number>;
  by_action: Record<string, number>;
};

/**
 * Ingest a new notification via the core RPC pipeline.
 * Calls `openhuman.notification_ingest`.
 *
 * Returns `{ id }` when the notification was persisted, or
 * `{ skipped: true, reason }` when the provider is disabled.
 */
export async function ingestNotification(payload: {
  provider: string;
  account_id?: string;
  title: string;
  body: string;
  raw_payload: Record<string, unknown>;
}): Promise<NotificationIngestResult> {
  log('ingestNotification provider=%s', payload.provider);
  const result = await callCoreRpc<NotificationIngestResult>({
    method: 'openhuman.notification_ingest',
    params: payload,
  });
  if (result.skipped) {
    log('ingestNotification skipped provider=%s reason=%s', payload.provider, result.reason);
  } else {
    log('ingestNotification created id=%s', result.id);
  }
  return result;
}

export async function getNotificationSettings(
  provider: string
): Promise<{
  provider: string;
  enabled: boolean;
  importance_threshold: number;
  route_to_orchestrator: boolean;
}> {
  const result = await callCoreRpc<{
    settings: {
      provider: string;
      enabled: boolean;
      importance_threshold: number;
      route_to_orchestrator: boolean;
    };
  }>({ method: 'openhuman.notification_settings_get', params: { provider } });
  return result.settings;
}

export async function setNotificationSettings(payload: {
  provider: string;
  enabled: boolean;
  importance_threshold: number;
  route_to_orchestrator: boolean;
}): Promise<void> {
  await callCoreRpc<{ ok: boolean }>({
    method: 'openhuman.notification_settings_set',
    params: payload,
  });
}

export async function dismissNotification(id: string): Promise<void> {
  log('dismissNotification id=%s', id);
  await callCoreRpc<{ ok: boolean }>({ method: 'openhuman.notification_dismiss', params: { id } });
}

export async function markNotificationActed(id: string): Promise<void> {
  log('markNotificationActed id=%s', id);
  await callCoreRpc<{ ok: boolean }>({
    method: 'openhuman.notification_mark_acted',
    params: { id },
  });
}

export async function fetchNotificationStats(): Promise<NotificationStats> {
  log('fetchNotificationStats');
  return callCoreRpc<NotificationStats>({ method: 'openhuman.notification_stats', params: {} });
}
