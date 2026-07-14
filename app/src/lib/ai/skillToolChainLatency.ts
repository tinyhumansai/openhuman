/**
 * Skill tool-chain latency observability (issue #4273, AC3).
 *
 * The acceptance criterion is that a complex skill tool chain "completes within
 * 60 seconds". We deliberately do NOT enforce that as a hard cap — clamping the
 * existing 120s tool timeout down to 60s would be a breaking behaviour change
 * that could cut off legitimately long chains. Instead we *measure* the wall
 * clock from the first tool call of a turn to the turn's completion and flag any
 * chain that overruns the target, so the budget is observable in logs/telemetry
 * and regressions surface without changing runtime behaviour.
 *
 * This module is pure and side-effect free (it takes an injectable clock) so it
 * can be unit-tested without sockets. `ChatRuntimeProvider` owns the single
 * instance and feeds it the canonical `tool_call` / `chat_done` / `chat_error`
 * events it already subscribes to.
 */

/** The AC3 budget: complex skill tool chains should finish within this. */
export const SKILL_TOOL_CHAIN_TARGET_MS = 60_000;

export interface ToolChainMeasurement {
  /** Thread the chain ran on. */
  threadId: string;
  /** Wall-clock from the first tool call to turn completion, in ms. */
  elapsedMs: number;
  /** Number of tool calls observed during the chain — counts every call, not
   *  unique tool names (the tracker is keyed only by chain id, not by tool). */
  toolCount: number;
  /** Whether the chain finished within {@link SKILL_TOOL_CHAIN_TARGET_MS}. */
  withinTarget: boolean;
  /** False when the turn ended via `chat_error` rather than `chat_done`. */
  ok: boolean;
}

export interface SkillToolChainLatencyTracker {
  /**
   * Record a tool call on a thread. The first call starts the chain timer; each
   * subsequent call increments the tool count. No-op shape until the next
   * `finishChain` resets the thread's state.
   */
  noteToolCall(threadId: string): void;
  /**
   * Close out a thread's chain and return its measurement, or `null` if no tool
   * call was ever seen for the thread (a pure text turn is not a "tool chain").
   */
  finishChain(threadId: string, opts?: { ok?: boolean }): ToolChainMeasurement | null;
  /** Drop all in-flight chain state (e.g. on provider unmount). */
  reset(): void;
}

interface ChainState {
  startedAt: number;
  toolCount: number;
}

/**
 * Create a tracker. `now` and `targetMs` are injectable for tests; production
 * uses `Date.now()` and {@link SKILL_TOOL_CHAIN_TARGET_MS}.
 */
export function createSkillToolChainLatencyTracker(
  now: () => number = () => Date.now(),
  targetMs: number = SKILL_TOOL_CHAIN_TARGET_MS
): SkillToolChainLatencyTracker {
  const chains = new Map<string, ChainState>();

  return {
    noteToolCall(threadId: string): void {
      const existing = chains.get(threadId);
      if (existing) {
        existing.toolCount += 1;
        return;
      }
      chains.set(threadId, { startedAt: now(), toolCount: 1 });
    },

    finishChain(threadId: string, opts?: { ok?: boolean }): ToolChainMeasurement | null {
      const state = chains.get(threadId);
      if (!state) return null;
      chains.delete(threadId);
      const elapsedMs = Math.max(0, now() - state.startedAt);
      return {
        threadId,
        elapsedMs,
        toolCount: state.toolCount,
        withinTarget: elapsedMs <= targetMs,
        ok: opts?.ok ?? true,
      };
    },

    reset(): void {
      chains.clear();
    },
  };
}
