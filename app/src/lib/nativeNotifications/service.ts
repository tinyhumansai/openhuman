import debug from 'debug';

import { socketService } from '../../services/socketService';
import { store } from '../../store';
import {
  type NotificationAction,
  type NotificationCategory,
  type NotificationItem,
  notificationReceived,
} from '../../store/notificationSlice';
import { ensureNotificationPermission, showNativeNotification } from './tauriBridge';

const log = debug('native-notifications');

let started = false;

// Retain listener references so stopNativeNotificationsService can remove them.
let chatDoneListener: ((...args: unknown[]) => void) | null = null;
let chatErrorListener: ((...args: unknown[]) => void) | null = null;
let coreNotificationListener: ((...args: unknown[]) => void) | null = null;
let workspaceChangedListener: ((...args: unknown[]) => void) | null = null;
let disconnectListener: ((...args: unknown[]) => void) | null = null;

/**
 * Opaque handle of the workspace the core is serving, or `null` while that is
 * unknown (issue #5966).
 *
 * Seeded by the core the moment this client connects and updated whenever the
 * active workspace changes, both over `workspace_changed`. A handle, never a
 * path — the core hashes `workspace_dir` before sending it, since the path is
 * under the user's home directory and this reaches every connected client.
 */
let activeWorkspace: string | null = null;

/**
 * Revision of the workspace transition `activeWorkspace` came from.
 *
 * The core seeds each client on connect from one task and broadcasts switches
 * from another, so a snapshot resolved before a switch can be delivered after
 * its broadcast. Keeping the highest revision seen and discarding anything
 * older stops a late snapshot talking this client back into the previous
 * workspace.
 */
let activeWorkspaceRevision = 0;

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
  // Optional action buttons (e.g. meeting auto-join prompt, issue #3507).
  // The Rust core serializes these camelCase, so the shape already matches
  // the Redux `NotificationAction` type — pass through verbatim.
  actions?: NotificationAction[];
  // Opaque handle of the workspace this notification belongs to, absent when
  // it is not workspace-bound (issue #5966). See `isForActiveWorkspace`.
  workspace?: string | null;
  // Workspace revision the core's announcement gate checked against, set only
  // when `workspace` is. See `isForActiveWorkspace`.
  workspace_revision?: number;
}

interface WorkspaceChangedPayload {
  workspace?: string | null;
  revision?: number;
}

/**
 * Whether a core notification belongs where this client is looking.
 *
 * The core already refuses to broadcast a notification from a workspace the
 * user has switched away from, but it decides that by resolving the active
 * workspace and then sending — two steps, so a switch in between can still
 * let one through. Re-checking here turns that publish-time boolean into an
 * identity the receiver can verify, which is what actually closes the window.
 *
 * Both unknowns pass rather than fail:
 *
 * - a payload with no `workspace` is not workspace-bound (cron, webhook,
 *   sub-agent, rejected API key) and applies wherever it lands — as does one
 *   persisted before the field existed;
 * - an unknown `activeWorkspace` means the seed has not arrived or the core
 *   could not resolve it, and dropping everything in that state would
 *   silently swallow every notification for the rest of the session.
 *
 * Failing open here is the opposite of the core's fail-closed gate, and
 * deliberately so: the core is the one deciding whether to *send*, this is a
 * second check on something already sent past that gate.
 *
 * The revision separates the two ways a handle can mismatch. This event and
 * `workspace_changed` are broadcast by separate tasks, so a notification for
 * the workspace the user just switched *to* can arrive before the switch that
 * announces it. A payload stamped with a revision newer than this client's
 * means the client is simply behind: the core verified that workspace was
 * active, so accept it and catch up rather than dropping a valid alert. Only a
 * mismatch at a revision this client has already caught up to is the stale
 * case the check exists for.
 */
function isForActiveWorkspace(payload: CoreNotificationPayload): boolean {
  if (!payload.workspace || !activeWorkspace) return true;
  if (payload.workspace === activeWorkspace) return true;
  const revision = payload.workspace_revision;
  if (typeof revision === 'number' && revision > activeWorkspaceRevision) {
    log(
      '[socket] core_notification is ahead of this client (rev=%d > %d); adopting %s',
      revision,
      activeWorkspaceRevision,
      payload.workspace
    );
    activeWorkspace = payload.workspace;
    activeWorkspaceRevision = revision;
    return true;
  }
  return false;
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
  log(
    '[dispatch] category=%s id=%s enabled=%s focused=%s',
    category,
    item.id,
    prefs[category],
    windowIsFocused()
  );
  if (!prefs[category]) {
    log('category %s disabled, skipping', category);
    return;
  }
  const timestamp = timestampOverride && timestampOverride > 0 ? timestampOverride : Date.now();
  const full: NotificationItem = { ...item, category, timestamp, read: false };
  log('[dispatch] enqueue id=%s title=%s', full.id, full.title);
  store.dispatch(notificationReceived(full));
  // Only fire OS-level banner when the user isn't already looking at the
  // window — otherwise the in-app center is enough and a native toast is
  // redundant noise.
  if (!windowIsFocused()) {
    log('[dispatch] window unfocused, firing native banner id=%s', full.id);
    void showNativeNotification({ title: full.title, body: full.body });
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

  // Request OS notification permission early so native banners can fire.
  // Fire-and-forget — permission state is logged for diagnostics.
  void ensureNotificationPermission().then(granted => {
    log('notification permission ensured: granted=%s', granted);
  });

  chatDoneListener = (...args: unknown[]) => {
    const p = (args[0] ?? {}) as ChatDonePayload;
    log('[socket] chat_done');
    dispatchAndMaybeBanner('agents', {
      id: `chat_done:${p.thread_id ?? 'unknown'}:${p.request_id ?? Date.now()}`,
      title: 'Agent reply ready',
      body: truncate(p.full_response?.trim() || 'Agent finished processing.', 160),
      deepLink: '/chat',
    });
  };

  chatErrorListener = (...args: unknown[]) => {
    const p = (args[0] ?? {}) as ChatErrorPayload;
    log('[socket] chat_error');
    dispatchAndMaybeBanner('system', {
      id: `chat_error:${p.thread_id ?? 'unknown'}:${p.request_id ?? Date.now()}`,
      title: 'Agent error',
      body: truncate(p.message || 'An error occurred during inference.', 160),
      deepLink: '/chat',
    });
  };

  // Core-originated notifications (cron completions, webhook failures,
  // sub-agent completions) bridged over socket.io from the Rust event
  // bus. See src/openhuman/desktop/notifications/bus.rs.
  coreNotificationListener = (...args: unknown[]) => {
    const p = (args[0] ?? {}) as CoreNotificationPayload;
    log('[socket] core_notification id=%s category=%s', p.id, p.category);
    if (!p.id || !p.title) {
      log('[socket] core_notification missing id/title dropped');
      return;
    }
    if (!isForActiveWorkspace(p)) {
      log(
        '[socket] core_notification id=%s dropped: workspace=%s active=%s',
        p.id,
        p.workspace,
        activeWorkspace
      );
      return;
    }
    const serverTs = p.timestamp_ms && p.timestamp_ms > 0 ? p.timestamp_ms : Date.now();
    dispatchAndMaybeBanner(
      p.category,
      {
        id: p.id,
        title: truncate(p.title, 120),
        body: truncate(p.body ?? '', 160),
        deepLink: p.deep_link ?? undefined,
        actions: p.actions,
      },
      serverTs
    );
  };

  // The core emits this once when this client connects and again on every
  // workspace switch, so `activeWorkspace` is current without polling and
  // without this module having to resolve anything itself (#5966).
  workspaceChangedListener = (...args: unknown[]) => {
    const p = (args[0] ?? {}) as WorkspaceChangedPayload;
    if (typeof p.workspace !== 'string' || !p.workspace) {
      log('[socket] workspace_changed without a handle ignored');
      return;
    }
    // A payload without a revision is treated as revision 0 and therefore
    // only accepted before anything else has been seen — an old core cannot
    // overwrite a newer switch from a current one.
    const revision = typeof p.revision === 'number' ? p.revision : 0;
    if (activeWorkspace !== null && revision < activeWorkspaceRevision) {
      log(
        '[socket] workspace_changed rev=%d discarded, already at rev=%d',
        revision,
        activeWorkspaceRevision
      );
      return;
    }
    activeWorkspaceRevision = revision;
    if (activeWorkspace === p.workspace) return;
    log('[socket] workspace_changed %s -> %s (rev=%d)', activeWorkspace, p.workspace, revision);
    activeWorkspace = p.workspace;
  };

  disconnectListener = (...args: unknown[]) => {
    const reason = typeof args[0] === 'string' ? args[0] : 'unknown';
    log('[socket] disconnect reason=%s', reason);
    dispatchAndMaybeBanner('system', {
      id: `socket_disconnect:${Date.now()}`,
      title: 'Connection lost',
      body: `OpenHuman lost its connection to the core service (${truncate(reason, 80)}).`,
    });
  };

  socketService.on('chat_done', chatDoneListener);
  socketService.on('chat_error', chatErrorListener);
  socketService.on('core_notification', coreNotificationListener);
  socketService.on('workspace_changed', workspaceChangedListener);
  socketService.on('disconnect', disconnectListener);

  log(
    'started — subscribed to chat_done, chat_error, core_notification, workspace_changed, disconnect'
  );
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
  if (workspaceChangedListener) {
    socketService.off('workspace_changed', workspaceChangedListener);
    workspaceChangedListener = null;
  }
  // Forget which workspace was active. The core seeds a client when its
  // *socket* connects, not when this service starts, so a stop → workspace
  // switch → restart over one live socket would otherwise resume holding the
  // pre-switch handle and drop every notification for the workspace the user
  // is now in.
  activeWorkspace = null;
  activeWorkspaceRevision = 0;
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
  if (!isForActiveWorkspace(payload)) return;
  const serverTs =
    payload.timestamp_ms && payload.timestamp_ms > 0 ? payload.timestamp_ms : Date.now();
  dispatchAndMaybeBanner(
    payload.category,
    {
      id: payload.id,
      title: truncate(payload.title, 120),
      body: truncate(payload.body ?? '', 160),
      deepLink: payload.deep_link ?? undefined,
      actions: payload.actions,
    },
    serverTs
  );
}

/**
 * Exposed for tests — set the workspace this client believes is active, as if
 * a `workspace_changed` had arrived.
 */
export function __setActiveWorkspaceForTests(workspace: string | null, revision = 0): void {
  activeWorkspace = workspace;
  activeWorkspaceRevision = revision;
}

/** Exposed for tests — resets module singletons between runs. */
export function __resetForTests(): void {
  started = false;
  chatDoneListener = null;
  chatErrorListener = null;
  coreNotificationListener = null;
  workspaceChangedListener = null;
  disconnectListener = null;
  activeWorkspace = null;
}
