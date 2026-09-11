/**
 * Session-scoped pins mapping a proactive *conversation surface* id to the
 * visible chat thread it first resolved to. A conversation surface — today only
 * the realtime voice session's `proactive:voice` — delivers many sequential
 * turns under one synthetic id; pinning keeps them all in one thread instead of
 * spawning a fresh thread per turn.
 *
 * This lives at module scope (rather than inside a `ChatRuntimeProvider` ref) so
 * the realtime voice hook can clear the voice pin when a new session begins — a
 * client-only lifecycle event `ChatRuntimeProvider` cannot observe — without
 * prop/context plumbing. Without that clear the pin would outlive its session:
 * a second voice session in the same app run would deliver into the first
 * session's thread, which the Human page no longer has selected, so the answer
 * would land in an off-screen thread. Single-process singleton; tests must reset
 * it with `clearAllProactiveThreadPins`.
 */

/**
 * The realtime voice session's synthetic proactive thread id (mirrors
 * `VOICE_CHAT_THREAD_ID` in `voice/realtime_harness.rs`). Every deferred voice
 * turn delivers its answer as a `proactive_message` under this one id.
 */
export const PROACTIVE_VOICE_THREAD_ID = 'proactive:voice';

/**
 * Whether a `proactive:` id names an ongoing *conversation surface* — many
 * sequential turns that all belong to a single chat — rather than a one-shot
 * interruption (morning brief, subconscious update, worker handoff). A
 * conversation surface pins the visible thread it first resolves to and reuses
 * it for the session; a one-shot keeps the fresh-or-create behaviour so it never
 * lands in the user's active chat (#3713). Today only realtime voice qualifies.
 */
export function isProactiveConversationSurface(incomingThreadId: string): boolean {
  return incomingThreadId === PROACTIVE_VOICE_THREAD_ID;
}

const pins = new Map<string, string>();

export const proactiveThreadPins = {
  /** The visible thread this surface is pinned to, or `undefined` if none. */
  get: (surfaceId: string): string | undefined => pins.get(surfaceId),
  /** Pin `surfaceId` to `threadId` for the rest of the session. */
  set: (surfaceId: string, threadId: string): void => {
    pins.set(surfaceId, threadId);
  },
  /**
   * Drop a single surface's pin so its next delivery re-resolves a fresh target.
   * Named `delete` (mirroring `Map.prototype.delete`) — it removes only the
   * given key, never the whole map. To wipe every pin (tests), use
   * `clearAllProactiveThreadPins`.
   */
  delete: (surfaceId: string): void => {
    pins.delete(surfaceId);
  },
};

/** Test-only: drop every pin so module state does not leak between tests. */
export function clearAllProactiveThreadPins(): void {
  pins.clear();
}
