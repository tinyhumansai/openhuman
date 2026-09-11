import debugFactory from 'debug';
import { type RefObject, useEffect } from 'react';

import { registerChatSurface } from '../../../providers/chatSurfaceHandlers';

const debug = debugFactory('openhuman:chat-surface');

/**
 * Claim ownership of a thread's write path for assistant-ui's runtime.
 *
 * `useOpenHumanExternalStore`'s `onNew`/`onCancel` do not implement sending —
 * they look the owning surface up in `chatSurfaceHandlers` and forward. This
 * hook is the registration half of that seam for the home chat surface.
 *
 * Why refs, and not the functions themselves: the chat surface re-creates its
 * send/stop closures on every render (they close over composer state that
 * changes per keystroke). Registering those directly would rewrite the registry
 * slot on every render — churn that, with `registerChatSurface`'s
 * still-own-the-slot cleanup guard, can drop the registration mid-turn. Binding
 * through refs instead makes the registered identity stable, so the effect can
 * depend on nothing but the thread id while still calling the latest
 * implementation.
 *
 * @param threadId The thread this surface currently owns, or `null` when none.
 * @param sendRef Ref to the surface's composer send function — the SAME entry
 *   point the Send button and Enter use, so the queued-follow-up vs. new-turn
 *   decision is made in one place rather than duplicated here.
 * @param stopRef Ref to the surface's cancel-in-flight-turn function.
 */
export function useChatSurfaceRegistration(
  threadId: string | null,
  sendRef: RefObject<((text?: string) => Promise<void>) | null>,
  stopRef: RefObject<(() => void) | null>,
  registerWithoutThread = false
): void {
  useEffect(() => {
    if (!threadId && !registerWithoutThread) return;
    debug('[chat] registering assistant-ui chat surface thread=%s', threadId);
    const dispose = registerChatSurface(threadId, {
      send: async (text: string) => {
        await sendRef.current?.(text);
      },
      cancel: async () => {
        stopRef.current?.();
      },
      // `reload` is deliberately absent. This surface has no
      // regenerate-the-last-turn path (`ai_regenerate` re-runs a failed
      // ARTIFACT, not a chat turn), and an `onReload` that silently does
      // nothing is worse than one the runtime can report as unavailable.
    });
    return () => {
      debug('[chat] unregistering assistant-ui chat surface thread=%s', threadId);
      dispose();
    };
  }, [registerWithoutThread, threadId, sendRef, stopRef]);
}
