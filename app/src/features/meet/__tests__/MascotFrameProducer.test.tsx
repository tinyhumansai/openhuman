import { listen } from '@tauri-apps/api/event';
import { act, cleanup, waitFor } from '@testing-library/react';
import { createElement } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import { MascotFrameProducer, sampleCanvasPixels } from '../MascotFrameProducer';

// @tauri-apps/api/event is mocked in setup.ts (listen → vi.fn()). Here we
// override it per-test to capture the event handlers so we can drive a fake
// `meet-video:bus-started` and assert how many mascot hosts mount.
type Listener = (event: { payload: unknown }) => void;

// The producer renders ManifestRiveMascot / RiveMascot (WebGL). Mock both leaf
// renderers to a plain <canvas> host so the frame's slot structure is
// assertable without the Rive runtime. Each records the face it was asked to
// render via a data attribute.
vi.mock('../../human/Mascot', async () => {
  const actual = await vi.importActual<typeof import('../../human/Mascot')>('../../human/Mascot');
  const stub = (props: { face?: string }) =>
    createElement('canvas', { 'data-face': props.face, width: 200, height: 200 });
  return { ...actual, ManifestRiveMascot: stub, RiveMascot: stub };
});

// Keep the manifest resolution deterministic + synchronous — the host tree
// mounts regardless (MascotStage falls back to RiveMascot when entry is null),
// so we just avoid a real network fetch.
vi.mock('../../human/Mascot/manifest/useMascotManifest', () => ({
  useMascotManifest: () => ({ manifest: null, entry: null, loading: false, error: null }),
}));

vi.mock('../../human/Mascot/manifest/manifestService', () => ({ findMascot: () => null }));

// Timings the component uses (kept in sync with MascotFrameProducer.tsx).
const GREETING_MS = 2500;
const SIGNOFF_MS = 1500;

/**
 * A fake `Worker` whose `postMessage({cmd:'start'})` records itself so the test
 * can fire a single tick on demand (invoking the producer's `onmessage`, which
 * calls `captureFrame`). Real timers/intervals are avoided — the test drives
 * ticks explicitly so the capture path runs deterministically under fake
 * timers.
 */
const workers: FakeWorker[] = [];
class FakeWorker {
  onmessage: ((e: { data: unknown }) => void) | null = null;
  started = false;
  terminated = false;
  constructor() {
    workers.push(this);
  }
  postMessage(msg: { cmd?: string }) {
    if (msg?.cmd === 'start') this.started = true;
    else if (msg?.cmd === 'stop') this.started = false;
  }
  terminate() {
    this.terminated = true;
  }
  /** Deliver one 'tick' to the producer, as the interval would. */
  tick() {
    this.onmessage?.({ data: 'tick' });
  }
}

/**
 * A fake `WebSocket` that opens synchronously (so `wsReadyRef` flips true) and
 * records every `send()` so the capture path's binary frame + JSON probe are
 * assertable.
 */
const sockets: FakeWebSocket[] = [];
class FakeWebSocket {
  static OPEN = 1;
  readyState = FakeWebSocket.OPEN;
  binaryType = 'arraybuffer';
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: ((e: unknown) => void) | null = null;
  sent: unknown[] = [];
  closed = false;
  constructor() {
    sockets.push(this);
    // Fire onopen on the next microtask so the effect that assigns
    // `ws.onopen = ...` has run before we invoke it.
    queueMicrotask(() => this.onopen?.());
  }
  send(data: unknown) {
    this.sent.push(data);
  }
  close() {
    this.closed = true;
    this.onclose?.();
  }
}

/**
 * A fake 2D context + OffscreenCanvas that satisfies the capture path:
 * gradient fill, per-cell drawImage, a pixel probe read, and a JPEG blob whose
 * `arrayBuffer()` resolves so the buffer is `send()`-able.
 */
class FakeOffscreenCanvas {
  drawImageCalls = 0;
  constructor(
    public width: number,
    public height: number
  ) {
    offscreens.push(this);
  }
  getContext() {
    const self = this;
    return {
      createRadialGradient: () => ({ addColorStop() {} }),
      fillStyle: '' as unknown,
      fillRect() {},
      drawImage() {
        self.drawImageCalls++;
      },
      getImageData: () => ({ data: [128, 128, 128, 255] }),
    };
  }
  convertToBlob() {
    return Promise.resolve({ size: 1234, arrayBuffer: () => Promise.resolve(new ArrayBuffer(8)) });
  }
}
const offscreens: FakeOffscreenCanvas[] = [];

/** Install the browser globals the ProducerSession effect touches. */
function installBrowserStubs() {
  workers.length = 0;
  sockets.length = 0;
  offscreens.length = 0;
  vi.stubGlobal('Worker', FakeWorker);
  vi.stubGlobal('WebSocket', FakeWebSocket);
  vi.stubGlobal('OffscreenCanvas', FakeOffscreenCanvas);
  if (!('createObjectURL' in URL)) {
    (URL as unknown as { createObjectURL: () => string }).createObjectURL = () => 'blob:x';
  }
  if (!('revokeObjectURL' in URL)) {
    (URL as unknown as { revokeObjectURL: () => void }).revokeObjectURL = () => {};
  }
  // jsdom does not implement HTMLMediaElement.play(); the silent keep-alive
  // audio the producer creates calls `.play().catch(...)`, so give it a
  // resolving stub.
  vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue(undefined);
}

/**
 * Wire the mocked `listen` so each event name captures its handler. Returns a
 * fn to fire a fake payload for a given event.
 */
function captureListeners() {
  const handlers = new Map<string, Listener>();
  vi.mocked(listen).mockImplementation((event: string, handler: unknown) => {
    handlers.set(event, handler as Listener);
    return Promise.resolve(vi.fn());
  });
  return {
    fire(event: string, payload: unknown) {
      const h = handlers.get(event);
      if (!h) throw new Error(`no listener registered for ${event}`);
      h({ payload });
    },
    has(event: string) {
      return handlers.has(event);
    },
  };
}

const SINGLE_MASCOT_STATE = {
  mascot: {
    color: 'yellow',
    voiceId: null,
    voiceGender: 'male',
    voiceUseLocaleDefault: false,
    selectedMascotId: 'tiny-mascot',
    secondaryMascotId: null,
    mascotVoices: {},
    customMascotGifUrl: null,
    customPrimaryColor: '#F7D145',
    customSecondaryColor: '#B23C05',
  },
};

const DUAL_MASCOT_STATE = { mascot: { ...SINGLE_MASCOT_STATE.mascot, secondaryMascotId: 'toshi' } };

describe('MascotFrameProducer', () => {
  beforeEach(() => {
    installBrowserStubs();
  });
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    // Restore the setup.ts default so other files that rely on the shared
    // `listen` mock (resolving to an unlisten fn) are unaffected.
    vi.mocked(listen).mockReset();
    vi.mocked(listen).mockResolvedValue(vi.fn());
  });

  it('renders nothing when no bus session is active', () => {
    const { container } = renderWithProviders(<MascotFrameProducer />);
    expect(container.firstChild).toBeNull();
  });

  it('mounts and unmounts without throwing', () => {
    expect(() => {
      const { unmount } = renderWithProviders(<MascotFrameProducer />);
      unmount();
    }).not.toThrow();
  });

  it('renders ONE mascot host for a single mascot', async () => {
    const bus = captureListeners();
    const { container } = renderWithProviders(<MascotFrameProducer />, {
      preloadedState: SINGLE_MASCOT_STATE,
    });

    await act(async () => {
      bus.fire('meet-video:bus-started', { requestId: 'r1', port: 55555 });
    });

    await waitFor(() => {
      expect(container.querySelectorAll('[data-mascot-slot]').length).toBe(1);
    });
    expect(container.querySelector('[data-mascot-slot="primary"]')).not.toBeNull();
    expect(container.querySelector('[data-mascot-slot="secondary"]')).toBeNull();
  });

  it('renders TWO mascot hosts for two distinct mascots', async () => {
    const bus = captureListeners();
    const { container } = renderWithProviders(<MascotFrameProducer />, {
      preloadedState: DUAL_MASCOT_STATE,
    });

    await act(async () => {
      bus.fire('meet-video:bus-started', { requestId: 'r2', port: 55556 });
    });

    await waitFor(() => {
      expect(container.querySelectorAll('[data-mascot-slot]').length).toBe(2);
    });
    expect(container.querySelector('[data-mascot-slot="primary"]')).not.toBeNull();
    expect(container.querySelector('[data-mascot-slot="secondary"]')).not.toBeNull();
  });

  it('ignores a bus-started payload with no port', async () => {
    const bus = captureListeners();
    const { container } = renderWithProviders(<MascotFrameProducer />, {
      preloadedState: SINGLE_MASCOT_STATE,
    });
    await act(async () => {
      bus.fire('meet-video:bus-started', { requestId: 'r0' });
    });
    // No port → guard returns early, session never set, nothing renders.
    expect(container.querySelector('[data-mascot-slot]')).toBeNull();
  });

  it('waves during greeting then transitions to active after GREETING_MS', async () => {
    vi.useFakeTimers();
    try {
      const bus = captureListeners();
      const { container } = renderWithProviders(<MascotFrameProducer />, {
        preloadedState: DUAL_MASCOT_STATE,
      });

      await act(async () => {
        bus.fire('meet-video:bus-started', { requestId: 'g1', port: 4100 });
        await vi.advanceTimersByTimeAsync(0);
      });

      // Greeting phase: both mascots wave.
      const facesDuringGreeting = Array.from(
        container.querySelectorAll('[data-mascot-slot] canvas')
      ).map(c => c.getAttribute('data-face'));
      expect(facesDuringGreeting).toEqual(['waving', 'waving']);

      // Advance past the greeting window → active phase. With no speaking
      // event yet, the active-slot mascot rests (listening) and the other
      // shows thinking — no longer both waving.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(GREETING_MS + 10);
      });

      const facesAfter = Array.from(container.querySelectorAll('[data-mascot-slot] canvas')).map(
        c => c.getAttribute('data-face')
      );
      expect(facesAfter).not.toEqual(['waving', 'waving']);
      expect(facesAfter).toContain('listening');
    } finally {
      vi.useRealTimers();
    }
  });

  it('holds the sign-off wave for SIGNOFF_MS then clears the session', async () => {
    vi.useFakeTimers();
    try {
      const bus = captureListeners();
      const { container } = renderWithProviders(<MascotFrameProducer />, {
        preloadedState: DUAL_MASCOT_STATE,
      });

      await act(async () => {
        bus.fire('meet-video:bus-started', { requestId: 's1', port: 4200 });
        await vi.advanceTimersByTimeAsync(GREETING_MS + 10);
      });
      // Session mounted (active phase).
      expect(container.querySelector('[data-mascot-slot]')).not.toBeNull();

      // bus-stopped → signoff phase; both mascots wave goodbye and the
      // session stays mounted through the grace window.
      await act(async () => {
        bus.fire('meet-video:bus-stopped', { requestId: 's1' });
        await vi.advanceTimersByTimeAsync(0);
      });
      const signoffFaces = Array.from(container.querySelectorAll('[data-mascot-slot] canvas')).map(
        c => c.getAttribute('data-face')
      );
      expect(signoffFaces).toEqual(['waving', 'waving']);
      // Still mounted just before the grace elapses.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(SIGNOFF_MS - 50);
      });
      expect(container.querySelector('[data-mascot-slot]')).not.toBeNull();

      // Grace elapsed → session cleared → producer renders nothing.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(100);
      });
      expect(container.querySelector('[data-mascot-slot]')).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it('re-arms a fresh greeting when bus-started fires during the sign-off grace', async () => {
    vi.useFakeTimers();
    try {
      const bus = captureListeners();
      const { container } = renderWithProviders(<MascotFrameProducer />, {
        preloadedState: DUAL_MASCOT_STATE,
      });
      await act(async () => {
        bus.fire('meet-video:bus-started', { requestId: 'a1', port: 4300 });
        await vi.advanceTimersByTimeAsync(GREETING_MS + 10);
        // Stop → enters signoff + arms the clear timer.
        bus.fire('meet-video:bus-stopped', { requestId: 'a1' });
        await vi.advanceTimersByTimeAsync(SIGNOFF_MS - 200);
        // A new session starts before the clear fires → clears the signoff
        // timer and restarts greeting.
        bus.fire('meet-video:bus-started', { requestId: 'a2', port: 4301 });
        await vi.advanceTimersByTimeAsync(0);
      });
      const faces = Array.from(container.querySelectorAll('[data-mascot-slot] canvas')).map(c =>
        c.getAttribute('data-face')
      );
      expect(faces).toEqual(['waving', 'waving']);

      // The old signoff clear must NOT fire now; session stays mounted.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(SIGNOFF_MS);
      });
      expect(container.querySelector('[data-mascot-slot]')).not.toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it('updates speaking state only for a matching requestId (gate)', async () => {
    vi.useFakeTimers();
    try {
      const bus = captureListeners();
      const { container } = renderWithProviders(<MascotFrameProducer />, {
        preloadedState: DUAL_MASCOT_STATE,
      });
      await act(async () => {
        bus.fire('meet-video:bus-started', { requestId: 'sp1', port: 4400 });
        await vi.advanceTimersByTimeAsync(0);
      });
      // Reach active phase so the face reflects speaking/slot rather than
      // the greeting wave.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(GREETING_MS + 10);
      });
      expect(
        Array.from(container.querySelectorAll('[data-mascot-slot] canvas')).map(c =>
          c.getAttribute('data-face')
        )
      ).not.toEqual(['waving', 'waving']);

      // Non-matching requestId is ignored: slot 1 speaking with the wrong id
      // must NOT flip any mascot to the speaking face.
      await act(async () => {
        bus.fire('meet-video:speaking-state', {
          requestId: 'STALE',
          speaking: true,
          activeMascotSlot: 1,
        });
        await vi.advanceTimersByTimeAsync(0);
      });
      let faces = Array.from(container.querySelectorAll('[data-mascot-slot] canvas')).map(c =>
        c.getAttribute('data-face')
      );
      expect(faces).not.toContain('speaking');

      // Matching requestId with slot 1 speaking → the secondary slot animates
      // (speaking), primary shows thinking.
      await act(async () => {
        bus.fire('meet-video:speaking-state', {
          requestId: 'sp1',
          speaking: true,
          activeMascotSlot: 1,
        });
        await vi.advanceTimersByTimeAsync(0);
      });
      faces = Array.from(container.querySelectorAll('[data-mascot-slot] canvas')).map(c =>
        c.getAttribute('data-face')
      );
      expect(faces).toEqual(['thinking', 'speaking']);

      // Slot 0 speaking → primary animates, secondary shows thinking. Also
      // exercises the `activeMascotSlot === 1 ? 1 : 0` default-to-0 branch.
      await act(async () => {
        bus.fire('meet-video:speaking-state', {
          requestId: 'sp1',
          speaking: true,
          activeMascotSlot: 0,
        });
        await vi.advanceTimersByTimeAsync(0);
      });
      faces = Array.from(container.querySelectorAll('[data-mascot-slot] canvas')).map(c =>
        c.getAttribute('data-face')
      );
      expect(faces).toEqual(['speaking', 'thinking']);
    } finally {
      vi.useRealTimers();
    }
  });

  it('ignores a speaking-state event with no payload', async () => {
    const bus = captureListeners();
    renderWithProviders(<MascotFrameProducer />, { preloadedState: SINGLE_MASCOT_STATE });
    await act(async () => {
      bus.fire('meet-video:bus-started', { requestId: 'np1', port: 4500 });
    });
    await waitFor(() => expect(bus.has('meet-video:speaking-state')).toBe(true));
    expect(() =>
      act(() => {
        bus.fire('meet-video:speaking-state', null);
      })
    ).not.toThrow();
  });

  it('captures and sends a single-mascot frame over the websocket on a worker tick', async () => {
    const bus = captureListeners();
    renderWithProviders(<MascotFrameProducer />, { preloadedState: SINGLE_MASCOT_STATE });

    await act(async () => {
      bus.fire('meet-video:bus-started', { requestId: 'cap1', port: 4600 });
    });
    // Wait for the WS/worker effect to wire up (worker registered, socket
    // open).
    await waitFor(() => expect(workers.length).toBeGreaterThan(0));
    await act(async () => {
      // let the queued microtask fire ws.onopen
      await Promise.resolve();
    });

    await act(async () => {
      workers[workers.length - 1].tick();
      // captureFrame is async (blob → arrayBuffer); flush its microtasks.
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    const ws = sockets[sockets.length - 1];
    // A binary ArrayBuffer frame was sent.
    const binary = ws.sent.filter(m => m instanceof ArrayBuffer);
    expect(binary.length).toBeGreaterThan(0);
    // The diagnostic JSON probe was also sent, and reports single-mascot.
    const jsonMsgs = ws.sent
      .filter((m): m is string => typeof m === 'string')
      .map(m => JSON.parse(m));
    expect(jsonMsgs.some(p => p.kind === 'producer-pixel-probe' && p.dualEnabled === false)).toBe(
      true
    );
    // Single-cell draw → drawMascotInCell called once.
    expect(offscreens[offscreens.length - 1].drawImageCalls).toBe(1);
  });

  it('captures and sends a dual-mascot frame (two-cell composite) on a worker tick', async () => {
    const bus = captureListeners();
    renderWithProviders(<MascotFrameProducer />, { preloadedState: DUAL_MASCOT_STATE });

    await act(async () => {
      bus.fire('meet-video:bus-started', { requestId: 'cap2', port: 4700 });
    });
    await waitFor(() => expect(workers.length).toBeGreaterThan(0));
    // Ensure both mascot slots are mounted so the dual (two-cell) branch runs.
    await waitFor(() =>
      expect(document.querySelectorAll('[data-mascot-slot="secondary"] canvas').length).toBe(1)
    );
    await act(async () => {
      await Promise.resolve();
    });

    await act(async () => {
      workers[workers.length - 1].tick();
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    const ws = sockets[sockets.length - 1];
    const jsonMsgs = ws.sent
      .filter((m): m is string => typeof m === 'string')
      .map(m => JSON.parse(m));
    expect(
      jsonMsgs.some(
        p => p.kind === 'producer-pixel-probe' && p.dualEnabled === true && p.secondaryMounted
      )
    ).toBe(true);
    // Two-cell draw → drawMascotInCell called twice (primary + secondary).
    expect(offscreens[offscreens.length - 1].drawImageCalls).toBe(2);
    expect(ws.sent.filter(m => m instanceof ArrayBuffer).length).toBeGreaterThan(0);
  });
});

// sampleCanvasPixels is still exported from the producer (re-exported from the
// compositor for back-compat); a light smoke check keeps that surface covered
// here. Full assertions live in mascotFrameCompositor.test.ts.
describe('sampleCanvasPixels (re-export)', () => {
  it('is re-exported and returns pixel stats', () => {
    const mockCtx = {
      getImageData: vi.fn().mockReturnValue({ data: [128, 128, 128, 255] }),
    } as unknown as OffscreenCanvasRenderingContext2D;
    expect(sampleCanvasPixels(mockCtx, 320, 240)).toMatchObject({ avgLuma: 128, sampleCount: 35 });
  });
});
