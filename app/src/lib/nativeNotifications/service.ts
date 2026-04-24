import debug from 'debug';

import { socketService } from '../../services/socketService';
import { store } from '../../store';
import {
  type NotificationCategory,
  type NotificationItem,
  notificationReceived,
} from '../../store/notificationSlice';
import { showNativeNotification } from './tauriBridge';

const log = debug('native-notifications');

let started = false;

// Retain listener references so stopNativeNotificationsService can remove them.
let chatDoneListener: ((...args: unknown[]) => void) | null = null;
let chatErrorListener: ((...args: unknown[]) => void) | null = null;
let coreNotificationListener: ((...args: unknown[]) => void) | null = null;
let disconnectListener: ((...args: unknown[]) => void) | null = null;

interface ChatDonePayload {
  thread_id?: string;
  request_id?: string;
  full_response?: string;
  rounds_used?: number;
}

interface ChatErrorPayload {
  thread_id?: string;
  request_id?: string;
  message?: string;
}

interface CoreNotificationPayload {
  id: string;
  category: NotificationCategory;
  title: string;
  body: string;
  deep_link?: string | null;
  timestamp_ms: number;
}

function windowIsFocused(): boolean {
  if (typeof document === 'undefined') return true;
  return document.hasFocus();
}

function dispatchAndMaybeBanner(
  category: NotificationCategory,
  item: Omit<NotificationItem, 'category' | 'timestamp' | 'read'>,
  timestampOverride?: number
): void {
  const prefs = store.getState().notifications.preferences;
  if (!prefs[category]) {
    log('category %s disabled, skipping', category);
    return;
  }
  const timestamp = timestampOverride && timestampOverride > 0 ? timestampOverride : Date.now();
  const full: NotificationItem = { ...item, category, timestamp, read: false };
  store.dispatch(notificationReceived(full));
  // Only fire OS-level banner when the user isn't already looking at the
  // window — otherwise the in-app center is enough and a native toast is
  // redundant noise.
  if (!windowIsFocused()) {
    void showNativeNotification({ title: full.title, body: full.body, tag: full.id });
  }
}

function truncate(input: string, max: number): string {
  if (input.length <= max) return input;
  return `${input.slice(0, max - 1)}…`;
}

/**
 * Subscribe to socket events that should surface as notifications (agent
 * completions, chat errors, core-originated events, connection drops).
 * Idempotent. Safe to call at app boot before the socket has connected —
 * the socketService queues listeners until the socket is ready.
 */
export function startNativeNotificationsService(): void {
  if (started) return;
  started = true;

  chatDoneListener = (...args: unknown[]) => {
    const p = (args[0] ?? {}) as ChatDonePayload;
    dispatchAndMaybeBanner('agents', {
      id: `chat_done:${p.thread_id ?? 'unknown'}:${p.request_id ?? Date.now()}`,
      title: 'Agent reply ready',
      body: truncate(p.full_response?.trim() || 'Agent finished processing.', 160),
      deepLink: '/chat',
    });
  };

  chatErrorListener = (...args: unknown[]) => {
    const p = (args[0] ?? {}) as ChatErrorPayload;
    dispatchAndMaybeBanner('system', {
      id: `chat_error:${p.thread_id ?? 'unknown'}:${p.request_id ?? Date.now()}`,
      title: 'Agent error',
      body: truncate(p.message || 'An error occurred during inference.', 160),
      deepLink: '/chat',
    });
  };

  // Core-originated notifications (cron completions, webhook failures,
  // sub-agent completions) bridged over socket.io from the Rust event
  // bus. See src/openhuman/notifications/bus.rs.
  coreNotificationListener = (...args: unknown[]) => {
    const p = (args[0] ?? {}) as CoreNotificationPayload;
    if (!p.id || !p.title) return;
    const serverTs = p.timestamp_ms && p.timestamp_ms > 0 ? p.timestamp_ms : Date.now();
    dispatchAndMaybeBanner(
      p.category,
      {
        id: p.id,
        title: truncate(p.title, 120),
        body: truncate(p.body ?? '', 160),
        deepLink: p.deep_link ?? undefined,
      },
      serverTs
    );
  };

  disconnectListener = (...args: unknown[]) => {
    const reason = typeof args[0] === 'string' ? args[0] : 'unknown';
    dispatchAndMaybeBanner('system', {
      id: `socket_disconnect:${Date.now()}`,
      title: 'Connection lost',
      body: `OpenHuman lost its connection to the core service (${truncate(reason, 80)}).`,
    });
  };

  socketService.on('chat_done', chatDoneListener);
  socketService.on('chat_error', chatErrorListener);
  socketService.on('core_notification', coreNotificationListener);
  socketService.on('disconnect', disconnectListener);

  log('started — subscribed to chat_done, chat_error, core_notification, disconnect');
}

export function stopNativeNotificationsService(): void {
  if (!started) return;

  if (chatDoneListener) {
    socketService.off('chat_done', chatDoneListener);
    chatDoneListener = null;
  }
  if (chatErrorListener) {
    socketService.off('chat_error', chatErrorListener);
    chatErrorListener = null;
  }
  if (coreNotificationListener) {
    socketService.off('core_notification', coreNotificationListener);
    coreNotificationListener = null;
  }
  if (disconnectListener) {
    socketService.off('disconnect', disconnectListener);
    disconnectListener = null;
  }

  started = false;
  log('stopped — all socket listeners removed');
}

/** Exposed for tests — dispatch as if a chat_done event arrived. */
export function __handleChatDoneForTests(payload: ChatDonePayload): void {
  dispatchAndMaybeBanner('agents', {
    id: `chat_done:${payload.thread_id ?? 'unknown'}:${payload.request_id ?? Date.now()}`,
    title: 'Agent reply ready',
    body: truncate(payload.full_response?.trim() || 'Agent finished processing.', 160),
    deepLink: '/chat',
  });
}

/** Exposed for tests — dispatch as if a core_notification arrived. */
export function __handleCoreNotificationForTests(payload: CoreNotificationPayload): void {
  if (!payload.id || !payload.title) return;
  const serverTs =
    payload.timestamp_ms && payload.timestamp_ms > 0 ? payload.timestamp_ms : Date.now();
  dispatchAndMaybeBanner(
    payload.category,
    {
      id: payload.id,
      title: truncate(payload.title, 120),
      body: truncate(payload.body ?? '', 160),
      deepLink: payload.deep_link ?? undefined,
    },
    serverTs
  );
}

/** Exposed for tests — resets module singletons between runs. */
export function __resetForTests(): void {
  started = false;
  chatDoneListener = null;
  chatErrorListener = null;
  coreNotificationListener = null;
  disconnectListener = null;
}
