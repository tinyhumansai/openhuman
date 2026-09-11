/**
 * The seam between assistant-ui's runtime write API and the surface that
 * actually owns sending.
 *
 * `onNew`/`onCancel`/`onReload` cannot be reimplemented inside the adapter.
 * The real send path (`Conversations.tsx`) is ~200 lines of orchestration —
 * attachment upload, thread creation, queue modes, the 600s silence timer,
 * lifecycle dispatches, analytics — and a second copy of it inside the runtime
 * adapter would be a fork that drifts, which is the precise way "assistant-ui
 * becomes the store" happens in practice.
 *
 * So the runtime does not own sending; it forwards. The active chat surface
 * registers its handlers here on mount, and the external-store adapter calls
 * through. Redux stays the source of truth and the send path stays singular.
 *
 * Registration is keyed by thread so two surfaces (home chat and the flows
 * copilot, which run different transports) cannot capture each other's turns.
 */

export interface ChatSurfaceHandlers {
  /** Send a new user turn. Mirrors whatever the surface's composer does. */
  send: (text: string) => Promise<void>;
  /** Cancel the in-flight turn, if the surface supports it. */
  cancel?: () => Promise<void>;
  /** Re-run the last turn, if the surface supports it. */
  reload?: () => Promise<void>;
}

const registry = new Map<string, ChatSurfaceHandlers>();
const UNSCOPED_SURFACE = '__openhuman_unscoped_chat_surface__';

/** Register the handlers for `threadId`; returns the unregister function. */
export function registerChatSurface(
  threadId: string | null,
  handlers: ChatSurfaceHandlers
): () => void {
  const key = threadId ?? UNSCOPED_SURFACE;
  registry.set(key, handlers);
  return () => {
    // Only clear if we still own the slot — a remount that re-registered first
    // must not be torn down by the outgoing instance's cleanup.
    if (registry.get(key) === handlers) registry.delete(key);
  };
}

/** The handlers registered for `threadId`, or `null` when no surface owns it. */
export function getChatSurface(threadId: string | null): ChatSurfaceHandlers | null {
  return registry.get(threadId ?? UNSCOPED_SURFACE) ?? null;
}

/** Test seam — drops every registration. */
export function __resetChatSurfaces(): void {
  registry.clear();
}
