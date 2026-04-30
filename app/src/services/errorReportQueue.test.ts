/**
 * @vitest-environment jsdom
 *
 * Tests for errorReportQueue — module-level error queue with no React/Redux/Sentry deps.
 *
 * Because the module holds mutable singleton state, we re-import it fresh in each
 * describe block via `vi.resetModules()` to isolate tests from each other.
 *
 * The source module calls `initGlobalListeners()` at load time, which reads `window`
 * — the explicit environment directive ensures jsdom is active even when Vitest picks
 * a different default for a given run.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// setup.ts already mocks @sentry/react and ../utils/config globally.
// We rely on those global mocks here.

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeReport(
  id: string,
  overrides: Partial<{
    title: string;
    message: string;
    source: 'react' | 'global' | 'manual';
    timestamp: number;
  }> = {}
) {
  return {
    id,
    timestamp: overrides.timestamp ?? Date.now(),
    source: overrides.source ?? ('manual' as const),
    title: overrides.title ?? `Error ${id}`,
    message: overrides.message ?? `Message for ${id}`,
    sentryEvent: null,
  };
}

// ---------------------------------------------------------------------------
// enqueueError / getErrors / dequeueError
// ---------------------------------------------------------------------------

describe('errorReportQueue — enqueue / dequeue / getErrors', () => {
  let eq: typeof import('./errorReportQueue');

  beforeEach(async () => {
    vi.resetModules();
    eq = await import('./errorReportQueue');
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('starts with an empty queue', () => {
    expect(eq.getErrors()).toEqual([]);
  });

  it('adds a report and returns it from getErrors', () => {
    const report = makeReport('r1');
    eq.enqueueError(report);

    const queue = eq.getErrors();
    expect(queue).toHaveLength(1);
    expect(queue[0].id).toBe('r1');
  });

  it('removes a report via dequeueError', () => {
    eq.enqueueError(makeReport('r1'));
    eq.enqueueError(makeReport('r2'));
    eq.dequeueError('r1');

    const queue = eq.getErrors();
    expect(queue).toHaveLength(1);
    expect(queue[0].id).toBe('r2');
  });

  it('is a no-op when dequeuing a non-existent ID', () => {
    eq.enqueueError(makeReport('r1'));
    eq.dequeueError('does-not-exist');

    expect(eq.getErrors()).toHaveLength(1);
  });

  it('preserves insertion order', () => {
    eq.enqueueError(makeReport('r1'));
    eq.enqueueError(makeReport('r2'));
    eq.enqueueError(makeReport('r3'));

    const ids = eq.getErrors().map(r => r.id);
    expect(ids).toEqual(['r1', 'r2', 'r3']);
  });

  it('caps queue at 10 items, keeping the most recent', () => {
    for (let i = 0; i < 12; i++) {
      // Each report has a unique title+message to avoid dedup
      eq.enqueueError(makeReport(`r${i}`, { title: `T${i}`, message: `M${i}` }));
    }

    const queue = eq.getErrors();
    expect(queue).toHaveLength(10);
    // Oldest two should have been dropped
    expect(queue[0].id).toBe('r2');
    expect(queue[9].id).toBe('r11');
  });
});

// ---------------------------------------------------------------------------
// Deduplication
// ---------------------------------------------------------------------------

describe('errorReportQueue — deduplication', () => {
  let eq: typeof import('./errorReportQueue');

  beforeEach(async () => {
    vi.useFakeTimers();
    vi.resetModules();
    eq = await import('./errorReportQueue');
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('suppresses a duplicate error within the dedup window (2 s)', () => {
    const base = makeReport('r1', { title: 'Same', message: 'Same' });
    eq.enqueueError(base);
    // Same title + message but different ID — should be deduped
    eq.enqueueError({ ...base, id: 'r1-dup' });

    expect(eq.getErrors()).toHaveLength(1);
  });

  it('allows the same error again after the dedup window expires', () => {
    const base = makeReport('r1', { title: 'Same', message: 'Same' });
    eq.enqueueError(base);

    // Advance past the 2000 ms DEDUP_WINDOW_MS
    vi.advanceTimersByTime(2001);

    eq.enqueueError({ ...base, id: 'r1-after' });
    expect(eq.getErrors()).toHaveLength(2);
  });

  it('allows distinct errors (different title/message) within the dedup window', () => {
    eq.enqueueError(makeReport('r1', { title: 'A', message: 'a' }));
    eq.enqueueError(makeReport('r2', { title: 'B', message: 'b' }));

    expect(eq.getErrors()).toHaveLength(2);
  });
});

// ---------------------------------------------------------------------------
// subscribe / notify
// ---------------------------------------------------------------------------

describe('errorReportQueue — subscribe', () => {
  let eq: typeof import('./errorReportQueue');

  beforeEach(async () => {
    vi.resetModules();
    eq = await import('./errorReportQueue');
  });

  it('calls subscriber when a report is enqueued', () => {
    const cb = vi.fn();
    eq.subscribe(cb);
    eq.enqueueError(makeReport('r1'));

    expect(cb).toHaveBeenCalledTimes(1);
  });

  it('calls subscriber when a report is dequeued', () => {
    eq.enqueueError(makeReport('r1'));
    const cb = vi.fn();
    eq.subscribe(cb);
    eq.dequeueError('r1');

    expect(cb).toHaveBeenCalledTimes(1);
  });

  it('does not call subscriber after unsubscribe', () => {
    const cb = vi.fn();
    const unsub = eq.subscribe(cb);
    unsub();
    eq.enqueueError(makeReport('r1'));

    expect(cb).not.toHaveBeenCalled();
  });

  it('supports multiple simultaneous subscribers', () => {
    const cb1 = vi.fn();
    const cb2 = vi.fn();
    eq.subscribe(cb1);
    eq.subscribe(cb2);
    eq.enqueueError(makeReport('r1'));

    expect(cb1).toHaveBeenCalledTimes(1);
    expect(cb2).toHaveBeenCalledTimes(1);
  });

  it('continues notifying other subscribers if one throws', () => {
    const bad = vi.fn(() => {
      throw new Error('subscriber boom');
    });
    const good = vi.fn();
    eq.subscribe(bad);
    eq.subscribe(good);

    expect(() => eq.enqueueError(makeReport('r1'))).not.toThrow();
    expect(good).toHaveBeenCalledTimes(1);
  });
});

// ---------------------------------------------------------------------------
// tagErrorSource
// ---------------------------------------------------------------------------

describe('errorReportQueue — tagErrorSource', () => {
  let eq: typeof import('./errorReportQueue');

  beforeEach(async () => {
    vi.resetModules();
    eq = await import('./errorReportQueue');
  });

  it('updates source on a queued report matched by sentryEvent.event_id', () => {
    const report = {
      ...makeReport('r1'),
      source: 'global' as const,
      sentryEvent: {
        event_id: 'sentry-abc',
        timestamp: Date.now() / 1000,
        platform: 'javascript',
        environment: 'test',
      },
    };
    eq.enqueueError(report);
    eq.tagErrorSource('sentry-abc', 'react', '<ErrorBoundary />');

    const updated = eq.getErrors().find(r => r.id === 'r1');
    expect(updated?.source).toBe('react');
    expect(updated?.componentStack).toBe('<ErrorBoundary />');
  });

  it('is a no-op when event_id is undefined', () => {
    eq.enqueueError(makeReport('r1'));
    expect(() => eq.tagErrorSource(undefined, 'react')).not.toThrow();
    expect(eq.getErrors()[0].source).toBe('manual');
  });

  it('is a no-op when event_id does not match any queued report', () => {
    eq.enqueueError(makeReport('r1'));
    expect(() => eq.tagErrorSource('unknown-id', 'react')).not.toThrow();
    expect(eq.getErrors()[0].source).toBe('manual');
  });
});

// ---------------------------------------------------------------------------
// registerSentrySender / sendToSentry
// ---------------------------------------------------------------------------

describe('errorReportQueue — sendToSentry', () => {
  let eq: typeof import('./errorReportQueue');

  beforeEach(async () => {
    vi.resetModules();
    eq = await import('./errorReportQueue');
  });

  it('returns false when no sender is registered', () => {
    const report = {
      ...makeReport('r1'),
      sentryEvent: {
        event_id: 'evt-1',
        timestamp: Date.now() / 1000,
        platform: 'javascript',
        environment: 'test',
      },
    };
    eq.enqueueError(report);

    expect(eq.sendToSentry(report)).toBe(false);
  });

  it('returns false when sentryEvent is null', () => {
    const sender = vi.fn();
    const report = makeReport('r1');
    eq.enqueueError(report);
    eq.registerSentrySender(sender);

    expect(eq.sendToSentry(report)).toBe(false);
    expect(sender).not.toHaveBeenCalled();
  });

  it('calls the registered sender and removes the report on success', () => {
    const sender = vi.fn();
    const sentryEvent = {
      event_id: 'evt-2',
      timestamp: Date.now() / 1000,
      platform: 'javascript',
      environment: 'test',
    };
    const report = { ...makeReport('r2'), sentryEvent };
    eq.enqueueError(report);
    eq.registerSentrySender(sender);

    const result = eq.sendToSentry(report);

    expect(result).toBe(true);
    expect(sender).toHaveBeenCalledWith(sentryEvent);
    expect(eq.getErrors().find(r => r.id === 'r2')).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// buildManualSentryEvent
// ---------------------------------------------------------------------------

describe('errorReportQueue — buildManualSentryEvent', () => {
  let eq: typeof import('./errorReportQueue');

  beforeEach(async () => {
    vi.resetModules();
    eq = await import('./errorReportQueue');
  });

  it('creates an event with the correct error type and value', () => {
    const evt = eq.buildManualSentryEvent({ type: 'TypeError', value: 'bad input' });

    expect(evt.exception?.values[0].type).toBe('TypeError');
    expect(evt.exception?.values[0].value).toBe('bad input');
  });

  it('sets platform to javascript', () => {
    const evt = eq.buildManualSentryEvent({ type: 'Error', value: 'msg' });
    expect(evt.platform).toBe('javascript');
  });

  it('includes optional tags when provided', () => {
    const evt = eq.buildManualSentryEvent({ type: 'Error', value: 'msg' }, { context: 'chat' });
    expect(evt.tags).toEqual({ context: 'chat' });
  });

  it('generates a non-empty event_id', () => {
    const evt = eq.buildManualSentryEvent({ type: 'Error', value: 'msg' });
    expect(evt.event_id).toBeTruthy();
    expect(typeof evt.event_id).toBe('string');
  });

  it('sets timestamp as a unix seconds value (numeric)', () => {
    const before = Date.now() / 1000;
    const evt = eq.buildManualSentryEvent({ type: 'Error', value: 'msg' });
    const after = Date.now() / 1000;

    expect(evt.timestamp).toBeGreaterThanOrEqual(before);
    expect(evt.timestamp).toBeLessThanOrEqual(after);
  });
});
