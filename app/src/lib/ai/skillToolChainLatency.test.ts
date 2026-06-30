import { describe, expect, it } from 'vitest';

import {
  createSkillToolChainLatencyTracker,
  SKILL_TOOL_CHAIN_TARGET_MS,
} from './skillToolChainLatency';

/** A clock whose value we advance by hand, so latency is deterministic. */
function fakeClock(start = 0) {
  let t = start;
  return {
    now: () => t,
    advance: (ms: number) => {
      t += ms;
    },
  };
}

describe('skillToolChainLatency', () => {
  it('returns null when a turn finished without any tool calls', () => {
    const tracker = createSkillToolChainLatencyTracker(() => 0);
    expect(tracker.finishChain('t1')).toBeNull();
  });

  it('measures elapsed time from the first tool call to completion', () => {
    const clock = fakeClock();
    const tracker = createSkillToolChainLatencyTracker(clock.now);

    tracker.noteToolCall('t1'); // chain starts at 0
    clock.advance(1_500);
    tracker.noteToolCall('t1'); // second tool, still same chain
    clock.advance(500);

    const m = tracker.finishChain('t1');
    expect(m).not.toBeNull();
    expect(m!.elapsedMs).toBe(2_000);
    expect(m!.toolCount).toBe(2);
    expect(m!.withinTarget).toBe(true);
    expect(m!.ok).toBe(true);
  });

  it('flags a chain that overruns the 60s target', () => {
    const clock = fakeClock();
    const tracker = createSkillToolChainLatencyTracker(clock.now);

    tracker.noteToolCall('t1');
    clock.advance(SKILL_TOOL_CHAIN_TARGET_MS + 1);

    const m = tracker.finishChain('t1');
    expect(m!.withinTarget).toBe(false);
    expect(m!.elapsedMs).toBe(SKILL_TOOL_CHAIN_TARGET_MS + 1);
  });

  it('treats exactly the target boundary as within target', () => {
    const clock = fakeClock();
    const tracker = createSkillToolChainLatencyTracker(clock.now);
    tracker.noteToolCall('t1');
    clock.advance(SKILL_TOOL_CHAIN_TARGET_MS);
    expect(tracker.finishChain('t1')!.withinTarget).toBe(true);
  });

  it('marks a chain ended by chat_error as not ok', () => {
    const tracker = createSkillToolChainLatencyTracker(() => 0);
    tracker.noteToolCall('t1');
    const m = tracker.finishChain('t1', { ok: false });
    expect(m!.ok).toBe(false);
  });

  it('tracks chains on separate threads independently', () => {
    const clock = fakeClock();
    const tracker = createSkillToolChainLatencyTracker(clock.now);

    tracker.noteToolCall('a'); // a starts at 0
    clock.advance(1_000);
    tracker.noteToolCall('b'); // b starts at 1000
    clock.advance(1_000);

    const a = tracker.finishChain('a');
    const b = tracker.finishChain('b');
    expect(a!.elapsedMs).toBe(2_000);
    expect(b!.elapsedMs).toBe(1_000);
  });

  it('does not double-count after a chain is finished', () => {
    const tracker = createSkillToolChainLatencyTracker(() => 0);
    tracker.noteToolCall('t1');
    expect(tracker.finishChain('t1')!.toolCount).toBe(1);
    // Same thread, fresh chain — must not resurrect the old state.
    expect(tracker.finishChain('t1')).toBeNull();
  });

  it('reset drops all in-flight chains', () => {
    const tracker = createSkillToolChainLatencyTracker(() => 0);
    tracker.noteToolCall('t1');
    tracker.reset();
    expect(tracker.finishChain('t1')).toBeNull();
  });
});
