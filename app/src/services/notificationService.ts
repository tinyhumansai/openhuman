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

/**
 * Ingest a new notification via the core RPC pipeline.
 * Calls `openhuman.notification_ingest`.
 *
 * This is typically called from the Tauri shell (Rust side) but can also be
 * invoked from the frontend for testing or manual ingestion.
 */
export async function ingestNotification(payload: {
  provider: string;
  account_id?: string;
  title: string;
  body: string;
  raw_payload: Record<string, unknown>;
}): Promise<{ id: string }> {
  log('ingestNotification provider=%s', payload.provider);
  const result = await callCoreRpc<{ id: string }>({
    method: 'openhuman.notification_ingest',
    params: payload,
  });
  log('ingestNotification created id=%s', result.id);
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
