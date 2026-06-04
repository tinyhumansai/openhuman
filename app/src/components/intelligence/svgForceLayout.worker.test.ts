/**
 * Drives the worker module directly with a stubbed `self`, since a real
 * Worker context can't run under the test env. Verifies it streams tick frames,
 * cools to an end, pins on drag, and halts on stop.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

interface FakeSelf {
  onmessage: ((e: { data: unknown }) => void) | null;
  postMessage: ReturnType<typeof vi.fn>;
}

let workerSelf: FakeSelf;

async function loadWorker() {
  workerSelf = { onmessage: null, postMessage: vi.fn() };
  vi.stubGlobal('self', workerSelf);
  vi.resetModules();
  await import('./svgForceLayout.worker');
}

function send(msg: unknown) {
  workerSelf.onmessage?.({ data: msg });
}

function initMsg(n: number) {
  return {
    type: 'init',
    nodes: Array.from({ length: n }, (_, i) => ({ x: i * 10, y: i * 10, r: 6 })),
    links: n > 1 ? [{ source: 1, target: 0 }] : [],
    cx: 550,
    cy: 320,
    alpha: 1,
  };
}

const ticks = () => workerSelf.postMessage.mock.calls.filter(c => c[0]?.type === 'tick');
const ended = () => workerSelf.postMessage.mock.calls.some(c => c[0]?.type === 'end');

describe('svgForceLayout.worker', () => {
  beforeEach(async () => {
    vi.useFakeTimers();
    await loadWorker();
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('streams a tick frame with a transferable positions buffer on init', () => {
    send(initMsg(3));
    const tick = ticks()[0];
    expect(tick).toBeTruthy();
    expect(tick[0].positions).toBeInstanceOf(Float32Array);
    expect(tick[0].positions.length).toBe(6); // 3 nodes × (x,y)
    expect(Array.isArray(tick[1])).toBe(true); // transfer list
  });

  it('cools to an end message and then stops ticking', () => {
    send(initMsg(2));
    vi.advanceTimersByTime(20000); // run the tick loop down to alphaMin
    expect(ended()).toBe(true);
    const afterEnd = workerSelf.postMessage.mock.calls.length;
    vi.advanceTimersByTime(5000);
    expect(workerSelf.postMessage.mock.calls.length).toBe(afterEnd); // no more ticks
  });

  it('halts the loop on stop (no further frames)', () => {
    send(initMsg(2));
    send({ type: 'stop' });
    const before = workerSelf.postMessage.mock.calls.length;
    vi.advanceTimersByTime(5000);
    expect(workerSelf.postMessage.mock.calls.length).toBe(before);
  });

  it('pins/unpins a node on drag and resumes ticking after it settles', () => {
    send(initMsg(2));
    vi.advanceTimersByTime(20000); // settle (loop idle)
    const before = workerSelf.postMessage.mock.calls.length;
    send({ type: 'drag', index: 0, x: 100, y: 100, fixed: true });
    vi.advanceTimersByTime(64); // a few frames
    expect(workerSelf.postMessage.mock.calls.length).toBeGreaterThan(before);
    send({ type: 'drag', index: 0, x: 0, y: 0, fixed: false }); // unpin — no throw
  });

  it('ignores a drag for an out-of-range index without throwing', () => {
    send(initMsg(2));
    expect(() => send({ type: 'drag', index: 99, x: 0, y: 0, fixed: true })).not.toThrow();
  });
});
