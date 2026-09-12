/**
 * The socket listeners themselves (issue #5966).
 *
 * `workspaceRouting.test.ts` drives the routing rule through the test seam and
 * proves the decision. This file proves the *wiring*: that starting the service
 * registers a `workspace_changed` listener, that the listener applies the
 * revision rule to what the core actually sends, that a `core_notification`
 * arriving over the socket is routed through the same check, and that stopping
 * the service forgets the workspace — the stop → switch → restart case where a
 * client would otherwise resume holding the pre-switch handle.
 *
 * The socket mock captures handlers instead of discarding them, which is the
 * only way to exercise those closures.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { socketService } from '../../../services/socketService';
import { store } from '../../../store';
import { setPreference } from '../../../store/notificationSlice';
import {
  __resetForTests,
  startNativeNotificationsService,
  stopNativeNotificationsService,
} from '../service';

vi.mock('../tauriBridge', () => ({
  showNativeNotification: vi.fn(),
  ensureNotificationPermission: vi.fn().mockResolvedValue(true),
}));

const handlers = new Map<string, (...args: unknown[]) => void>();
vi.mock('../../../services/socketService', () => ({
  socketService: {
    on: vi.fn((event: string, cb: (...args: unknown[]) => void) => {
      handlers.set(event, cb);
    }),
    off: vi.fn((event: string) => {
      handlers.delete(event);
    }),
  },
}));

const WS_A = 'ws_1111111111111111';
const WS_B = 'ws_2222222222222222';

const fire = (event: string, payload: unknown) => {
  const handler = handlers.get(event);
  if (!handler) throw new Error(`no listener registered for ${event}`);
  handler(payload);
};

const bound = (id: string, workspace: string, workspace_revision?: number) => ({
  id,
  category: 'system',
  title: 'MCP server parked',
  body: 'ac.inference.sh/mcp is not being retried.',
  timestamp_ms: 1,
  workspace,
  workspace_revision,
});

const items = () => store.getState().notifications.items;

describe('native notification socket listeners', () => {
  beforeEach(() => {
    stopNativeNotificationsService();
    __resetForTests();
    handlers.clear();
    vi.clearAllMocks();
    store.dispatch({ type: 'notifications/clearAll' });
    store.dispatch(setPreference({ category: 'system', enabled: true }));
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
  });

  it('subscribes to workspace_changed alongside the existing events', () => {
    startNativeNotificationsService();
    for (const event of [
      'chat_done',
      'chat_error',
      'core_notification',
      'workspace_changed',
      'disconnect',
    ]) {
      expect(handlers.has(event), `listener for ${event}`).toBe(true);
    }
  });

  it('is idempotent: a second start does not re-register', () => {
    startNativeNotificationsService();
    const calls = vi.mocked(socketService.on).mock.calls.length;
    startNativeNotificationsService();
    expect(vi.mocked(socketService.on).mock.calls.length).toBe(calls);
  });

  it('ignores a workspace_changed that carries no handle', () => {
    startNativeNotificationsService();
    fire('workspace_changed', { revision: 3 });
    fire('workspace_changed', {});
    // Still unknown, so a bound notification for any workspace is accepted.
    fire('core_notification', bound('n1', WS_B));
    expect(items()).toHaveLength(1);
  });

  it('adopts a switch and drops a notification from the workspace left behind', () => {
    startNativeNotificationsService();
    fire('workspace_changed', { workspace: WS_A, revision: 1 });
    fire('core_notification', bound('a1', WS_A));
    expect(items()).toHaveLength(1);

    fire('workspace_changed', { workspace: WS_B, revision: 2 });
    fire('core_notification', bound('a2', WS_A));
    expect(items()).toHaveLength(1);
    fire('core_notification', bound('b1', WS_B));
    expect(items()).toHaveLength(2);
  });

  /**
   * The connect-time seed and the switch broadcast travel on separate tasks,
   * so a seed resolved before a switch can arrive after it. Keeping the highest
   * revision stops it talking this client back into the previous workspace.
   */
  it('discards a workspace_changed older than what it already has', () => {
    startNativeNotificationsService();
    fire('workspace_changed', { workspace: WS_B, revision: 5 });
    fire('workspace_changed', { workspace: WS_A, revision: 4 });

    fire('core_notification', bound('stale-a', WS_A));
    expect(items()).toHaveLength(0);
    fire('core_notification', bound('b', WS_B));
    expect(items()).toHaveLength(1);
  });

  it('advances the revision on a repeat of the same workspace', () => {
    startNativeNotificationsService();
    fire('workspace_changed', { workspace: WS_A, revision: 1 });
    fire('workspace_changed', { workspace: WS_A, revision: 3 });
    // A switch to B at revision 2 is now older than what the client holds.
    fire('workspace_changed', { workspace: WS_B, revision: 2 });
    fire('core_notification', bound('a', WS_A));
    expect(items()).toHaveLength(1);
  });

  it('treats a payload without a revision as revision 0', () => {
    startNativeNotificationsService();
    fire('workspace_changed', { workspace: WS_A });
    fire('workspace_changed', { workspace: WS_B, revision: 1 });
    fire('core_notification', bound('b', WS_B));
    expect(items()).toHaveLength(1);
  });

  /**
   * The stop → switch → restart case. The core seeds a client when its socket
   * connects, not when this service starts, so a restart over a live socket
   * gets no fresh seed; the service must not resume holding the old handle.
   */
  it('forgets the workspace on stop so a restart cannot drop the new one', () => {
    startNativeNotificationsService();
    fire('workspace_changed', { workspace: WS_A, revision: 7 });
    stopNativeNotificationsService();
    expect(vi.mocked(socketService.off)).toHaveBeenCalledWith(
      'workspace_changed',
      expect.any(Function)
    );

    startNativeNotificationsService();
    // No seed arrives. A bound notification for B, unstamped, must not be
    // judged against the stale A.
    fire('core_notification', bound('b-after-restart', WS_B));
    expect(items()).toHaveLength(1);
  });

  it('stop is a no-op before start', () => {
    stopNativeNotificationsService();
    expect(vi.mocked(socketService.off)).not.toHaveBeenCalled();
  });
});
