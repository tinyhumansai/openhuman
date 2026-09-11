import { act, screen } from '@testing-library/react';
import { useLayoutEffect } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import { ChatMascotProvider, useChatMascot } from './ChatMascotContext';
import ChatMascotOverlay from './ChatMascotOverlay';
import { TRANSITION_MS } from './geometry';

const useHumanMascot = vi.fn((_opts: unknown) => ({
  face: 'idle',
  viseme: 'REST',
  visemeCode: 'sil',
}));
vi.mock('../useHumanMascot', () => ({ useHumanMascot: (opts: unknown) => useHumanMascot(opts) }));
vi.mock('../Mascot', () => ({
  CustomGifMascot: () => <div data-testid="mascot-gif" />,
  ManifestRiveMascot: () => <div data-testid="mascot-manifest" />,
  RiveMascot: () => <div data-testid="mascot-rive" />,
  getMascotPalette: () => ({ bodyFill: '#F7D145', neckShadowColor: '#B23C05' }),
  hexToArgbInt: () => 0,
}));
vi.mock('../Mascot/manifest/useMascotManifest', () => ({
  useMascotManifest: () => ({ manifest: null, entry: null, loading: false, error: null }),
}));

const DOCK_RECT = { left: 40, top: 500, width: 64, height: 64 };
const STAGE_RECT = { left: 800, top: 100, width: 400, height: 400 };

/** Mounts anchors whose rects are fixed, so the transform maths is deterministic. */
const Anchors = () => {
  const { dockRef, stageRef } = useChatMascot();
  useLayoutEffect(() => {
    const attach = (el: HTMLElement | null, rect: typeof DOCK_RECT) => {
      if (!el) return;
      el.getBoundingClientRect = () =>
        ({ ...rect, right: rect.left + rect.width, bottom: rect.top + rect.height }) as DOMRect;
    };
    attach(dockRef.current, DOCK_RECT);
    attach(stageRef.current, STAGE_RECT);
  });
  return (
    <>
      <div
        ref={node => {
          dockRef.current = node;
        }}
        data-testid="dock-anchor"
      />
      <div
        ref={node => {
          stageRef.current = node;
        }}
        data-testid="stage-anchor"
      />
    </>
  );
};

const renderOverlay = (expanded: boolean) =>
  renderWithProviders(
    <ChatMascotProvider>
      <Anchors />
      <ChatMascotOverlay />
    </ChatMascotProvider>,
    { preloadedState: { mascot: { chatMascotExpanded: expanded } } }
  );

/** Captured ResizeObserver callbacks, so tests can fire a layout change. */
const resizeObservers: Array<() => void> = [];

interface FrameRunner {
  /** Run the single queued frame with `timestamp`. */
  flushOne: (timestamp: number) => void;
  pending: () => number;
}

/** Replace rAF with a hand-driven queue so travel frames are deterministic. */
function driveAnimationFrames(): FrameRunner {
  let queue: FrameRequestCallback[] = [];
  vi.spyOn(window, 'requestAnimationFrame').mockImplementation(cb => {
    queue.push(cb);
    return queue.length;
  });
  return {
    flushOne: (timestamp: number) => {
      const due = queue;
      queue = [];
      due.forEach(cb => cb(timestamp));
    },
    pending: () => queue.length,
  };
}

/** Pull the translate offsets back out of a `translate3d(...) scale(...)`. */
function readTranslate(el: HTMLElement): { x: number; y: number } {
  const m = /translate3d\((-?[\d.]+)px, (-?[\d.]+)px/.exec(el.style.transform);
  if (!m) throw new Error(`no translate in transform: ${el.style.transform}`);
  return { x: Number(m[1]), y: Number(m[2]) };
}

const setReducedMotion = (reduce: boolean) => {
  window.matchMedia = vi
    .fn()
    .mockImplementation((query: string) => ({
      matches: reduce && query.includes('prefers-reduced-motion'),
      media: query,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
      onchange: null,
    })) as unknown as typeof window.matchMedia;
};

describe('ChatMascotOverlay', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setReducedMotion(false);
    resizeObservers.length = 0;
    vi.stubGlobal(
      'ResizeObserver',
      class {
        constructor(cb: () => void) {
          resizeObservers.push(cb);
        }
        observe() {}
        unobserve() {}
        disconnect() {}
      }
    );
  });

  afterEach(() => vi.unstubAllGlobals());

  it("lays out at the anchor's true size at rest, so Rive allocates a matching canvas", () => {
    // Load-bearing, not cosmetic. Rive sizes its canvas backing store from
    // `getBoundingClientRect()` — which includes ancestor transforms — and
    // `ResizeObserver` never fires on a transform change. A permanently
    // scaled box therefore gets a backing store matching whatever scale it was
    // measured at (docked: ~64px) and stretches that texture across the whole
    // stage forever. Measured before this fix: a 127x127 backing store behind a
    // 768px box. Changing the layout size is what makes Rive re-allocate.
    renderOverlay(false);

    const overlay = screen.getByTestId('chat-mascot-overlay');
    expect(overlay.style.width).toBe(`${DOCK_RECT.width}px`);
    expect(overlay.style.height).toBe(`${DOCK_RECT.width}px`);
    expect(overlay.style.transformOrigin).toBe('top left');
  });

  it('never scales the canvas at rest — translate only', () => {
    renderOverlay(true);
    const overlay = screen.getByTestId('chat-mascot-overlay');
    expect(overlay.style.transform).not.toContain('scale');
    expect(overlay.style.width).toBe('400px'); // the stage anchor's inscribed square
  });

  it('scales the render box down onto the dock when collapsed', () => {
    renderOverlay(false);

    // First placement never animates, so this is final on mount: laid out at the
    // dock's own size and simply moved there.
    const overlay = screen.getByTestId('chat-mascot-overlay');
    expect(overlay.style.transform).toBe('translate3d(40px, 500px, 0)');
    expect(overlay.style.width).toBe('64px');
    expect(overlay.dataset.expanded).toBe('false');
  });

  it('scales onto the stage anchor when expanded', () => {
    renderOverlay(true);

    const overlay = screen.getByTestId('chat-mascot-overlay');
    expect(overlay.style.transform).toBe('translate3d(800px, 100px, 0)');
    expect(overlay.style.width).toBe('400px');
    expect(overlay.dataset.expanded).toBe('true');
  });

  it('snaps rather than travelling when the user prefers reduced motion', () => {
    setReducedMotion(true);
    const raf = vi.spyOn(window, 'requestAnimationFrame');
    const { rerenderWithState } = renderOverlayWithToggle();

    act(() => rerenderWithState(true));

    expect(raf).not.toHaveBeenCalled();
    expect(screen.getByTestId('chat-mascot-overlay').style.transform).toBe(
      'translate3d(800px, 100px, 0)'
    );
  });

  it('renders exactly one mascot instance', () => {
    // The whole point of the overlay: a second instance would load the `.riv`
    // twice and turn the dock ⇄ stage travel into a crossfade.
    renderOverlay(true);

    expect(screen.getAllByTestId('mascot-rive')).toHaveLength(1);
  });

  it('stays hidden until an anchor is measurable, then lands on it', () => {
    // The dock can mount after the overlay. A ResizeObserver can only watch
    // elements that already exist, so without the poll a late anchor would
    // leave the mascot parked off-screen for good.
    vi.useFakeTimers();
    try {
      // The overlay stays FIRST in both trees on purpose: appending the anchor
      // after it keeps its position stable, so React updates it in place. Insert
      // the anchor *before* it instead and React unmounts and remounts the
      // overlay, whose fresh layout effect then measures the anchor directly —
      // the poll would never run and this test would pass for the wrong reason.
      const tree = (withAnchor: boolean) => (
        <ChatMascotProvider>
          <ChatMascotOverlay />
          {withAnchor ? <Anchors /> : null}
        </ChatMascotProvider>
      );

      const { rerender } = renderWithProviders(tree(false), {
        preloadedState: { mascot: { chatMascotExpanded: false } },
      });

      const overlay = screen.getByTestId('chat-mascot-overlay');
      expect(overlay.style.opacity).toBe('0');

      // The dock arrives. The overlay's layout effect does not re-run (its deps
      // are all stable refs) — only the poll can notice.
      rerender(tree(true));
      expect(overlay.style.opacity).toBe('0');

      act(() => void vi.advanceTimersByTime(200));

      expect(screen.getByTestId('chat-mascot-overlay')).toBe(overlay);
      expect(overlay.style.transform).toBe('translate3d(40px, 500px, 0)');
      expect(overlay.style.opacity).toBe('1');
    } finally {
      vi.useRealTimers();
    }
  });

  it('stops polling for an anchor that never arrives', () => {
    // A surface that mounts the overlay without a dock must not leave a timer
    // ticking for the rest of the session.
    vi.useFakeTimers();
    try {
      const clearInterval = vi.spyOn(window, 'clearInterval');
      renderWithProviders(
        <ChatMascotProvider>
          <ChatMascotOverlay />
        </ChatMascotProvider>,
        { preloadedState: { mascot: { chatMascotExpanded: false } } }
      );

      act(() => void vi.advanceTimersByTime(4_000));

      expect(clearInterval).toHaveBeenCalled();
      expect(screen.getByTestId('chat-mascot-overlay').style.opacity).toBe('0');
    } finally {
      vi.useRealTimers();
    }
  });

  it('travels from the dock to the stage, landing exactly on the anchor', () => {
    // The centrepiece animation. Drives rAF by hand so each frame's transform is
    // deterministic rather than wall-clock dependent.
    const frames: FrameRunner = driveAnimationFrames();
    let now = 0;
    vi.spyOn(window.performance, 'now').mockImplementation(() => now);

    const { store } = renderOverlay(false);
    const overlay = screen.getByTestId('chat-mascot-overlay');
    const docked = overlay.style.transform;

    act(() => {
      store.dispatch({ type: 'mascot/setChatMascotExpanded', payload: true });
    });

    // Halfway: between the two anchors, and hinted to the compositor.
    now = TRANSITION_MS / 2;
    act(() => frames.flushOne(now));
    const mid = readTranslate(overlay);
    expect(mid.x).toBeGreaterThan(DOCK_RECT.left);
    expect(mid.x).toBeLessThan(STAGE_RECT.left);
    expect(overlay.style.willChange).toBe('transform');

    // Past the end: lands on the stage anchor and releases the compositor hint.
    now = TRANSITION_MS + 1;
    act(() => frames.flushOne(now));
    expect(overlay.style.transform).not.toBe(docked);
    // Landed: real layout size, translate only, compositor hint released.
    expect(overlay.style.transform).toBe('translate3d(800px, 100px, 0)');
    expect(overlay.style.width).toBe('400px');
    expect(overlay.style.willChange).toBe('auto');
  });

  it('cancels an in-flight travel when the mascot unmounts', () => {
    const frames = driveAnimationFrames();
    const cancel = vi.spyOn(window, 'cancelAnimationFrame');

    const { store, unmount } = renderOverlay(false);
    act(() => {
      store.dispatch({ type: 'mascot/setChatMascotExpanded', payload: true });
    });
    expect(frames.pending()).toBeGreaterThan(0);

    unmount();

    expect(cancel).toHaveBeenCalled();
  });

  it('re-settles onto its anchor when the layout moves underneath it', () => {
    // The dock rides the composer, which moves whenever a draft grows or an
    // attachment strip appears. Nothing resizes the dock itself, so this is
    // driven by the observer on its ancestors.
    renderOverlay(false);
    const overlay = screen.getByTestId('chat-mascot-overlay');
    expect(readTranslate(overlay)).toEqual({ x: DOCK_RECT.left, y: DOCK_RECT.top });

    DOCK_RECT.top = 320;
    act(() => resizeObservers.forEach(cb => cb()));

    expect(readTranslate(overlay)).toEqual({ x: DOCK_RECT.left, y: 320 });
    DOCK_RECT.top = 500;
  });

  it('leaves the mascot alone mid-travel when the layout moves', () => {
    // The travel loop is already re-measuring every frame; a second writer would
    // fight it and make the mascot stutter.
    const frames = driveAnimationFrames();
    let now = 0;
    vi.spyOn(window.performance, 'now').mockImplementation(() => now);

    const { store } = renderOverlay(false);
    const overlay = screen.getByTestId('chat-mascot-overlay');
    act(() => {
      store.dispatch({ type: 'mascot/setChatMascotExpanded', payload: true });
    });
    now = TRANSITION_MS / 2;
    act(() => frames.flushOne(now));
    const midTravel = overlay.style.transform;

    act(() => window.dispatchEvent(new Event('resize')));

    expect(overlay.style.transform).toBe(midTravel);
  });

  it('only speaks replies while the stage is open', () => {
    // A docked mascot must not start talking over a text conversation just
    // because the persisted preference defaults on.
    renderWithProviders(
      <ChatMascotProvider>
        <Anchors />
        <ChatMascotOverlay />
      </ChatMascotProvider>,
      { preloadedState: { mascot: { chatMascotExpanded: false, speakReplies: true } } }
    );

    expect(useHumanMascot).toHaveBeenCalledWith(expect.objectContaining({ speakReplies: false }));
  });

  it('speaks replies once expanded with the preference on', () => {
    renderWithProviders(
      <ChatMascotProvider>
        <Anchors />
        <ChatMascotOverlay />
      </ChatMascotProvider>,
      { preloadedState: { mascot: { chatMascotExpanded: true, speakReplies: true } } }
    );

    expect(useHumanMascot).toHaveBeenCalledWith(expect.objectContaining({ speakReplies: true }));
  });
});

/** Render collapsed, then flip the store so the travel path is exercised. */
function renderOverlayWithToggle() {
  const utils = renderOverlay(false);
  return {
    ...utils,
    rerenderWithState: (expanded: boolean) => {
      utils.store.dispatch({ type: 'mascot/setChatMascotExpanded', payload: expanded });
    },
  };
}
