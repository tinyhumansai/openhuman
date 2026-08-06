import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { Gradient } from './meshGradient';

/**
 * Teardown safety for the WebGL mesh gradient (issue #5160).
 *
 * `Gradient` drives three async chains that all outlive a single React commit:
 * the `waitForCssVars` rAF retry loop, the `animate` rAF loop, and the 3s
 * deferred `isLoaded` class. `<MeshGradient />` unmounts on every theme switch
 * and backdrop change, so those callbacks used to fire against a canvas React
 * had already removed — `this.el.parentElement` was `null` and the
 * `.classList.add()` threw the "Cannot read properties of null (reading
 * 'classList')" family Sentry grouped across five bundles.
 *
 * These tests drive the lib directly (no WebGL): they set `el` by hand and
 * exercise the lifecycle entry points, which is where the leak lived.
 */
describe('Gradient teardown (#5160)', () => {
  let wrapper: HTMLDivElement;
  let canvas: HTMLCanvasElement;
  let rafQueue: Map<number, FrameRequestCallback>;
  let nextRafHandle: number;
  let cancelled: number[];

  beforeEach(() => {
    vi.useFakeTimers();
    wrapper = document.createElement('div');
    canvas = document.createElement('canvas');
    canvas.id = 'mesh-gradient';
    wrapper.appendChild(canvas);
    document.body.appendChild(wrapper);

    rafQueue = new Map();
    nextRafHandle = 0;
    cancelled = [];
    vi.spyOn(window, 'requestAnimationFrame').mockImplementation(callback => {
      nextRafHandle += 1;
      rafQueue.set(nextRafHandle, callback);
      return nextRafHandle;
    });
    vi.spyOn(window, 'cancelAnimationFrame').mockImplementation(handle => {
      cancelled.push(handle);
      rafQueue.delete(handle);
    });
  });

  afterEach(() => {
    wrapper.remove();
    vi.restoreAllMocks();
    vi.useRealTimers();
  });

  /** Runs whatever is queued right now, exactly like a single browser frame. */
  function flushFrame() {
    const pending = [...rafQueue.entries()];
    rafQueue.clear();
    for (const [, callback] of pending) callback(performance.now());
  }

  function mountedGradient() {
    const gradient = new Gradient();
    gradient.el = canvas;
    return gradient;
  }

  it('marks the wrapper loaded when the canvas is still mounted 3s later', () => {
    const gradient = mountedGradient();

    gradient.addIsLoadedClass();
    expect(canvas.classList.contains('isLoaded')).toBe(true);

    vi.advanceTimersByTime(3000);
    expect(wrapper.classList.contains('isLoaded')).toBe(true);
  });

  it('cancels the deferred isLoaded class when disconnect() runs first', () => {
    const gradient = mountedGradient();

    gradient.addIsLoadedClass();
    // React unmounts the component well inside the 3s window. The canvas is
    // deliberately left in the DOM here so the assertion pins the *timer being
    // cancelled* rather than the detached-node guard below it.
    gradient.disconnect();

    expect(() => vi.advanceTimersByTime(3000)).not.toThrow();
    expect(wrapper.classList.contains('isLoaded')).toBe(false);
  });

  it('skips the deferred isLoaded class when the canvas was detached without a disconnect', () => {
    const gradient = mountedGradient();

    gradient.addIsLoadedClass();
    // Node pulled out from under us — `el.parentElement` is now null, which is
    // the exact dereference Sentry reported.
    canvas.remove();

    expect(() => vi.advanceTimersByTime(3000)).not.toThrow();
    expect(wrapper.classList.contains('isLoaded')).toBe(false);
  });

  it('stops the waitForCssVars retry loop after disconnect()', () => {
    const gradient = mountedGradient();
    // No `--gradient-color-1` yet, so waitForCssVars re-schedules itself.
    gradient.computedCanvasStyle = { getPropertyValue: () => '' } as unknown as CSSStyleDeclaration;

    gradient.waitForCssVars();
    expect(rafQueue.size).toBe(1);

    gradient.disconnect();
    expect(rafQueue.size).toBe(0);

    // Even a frame that was already dispatched must not restart the chain.
    gradient.waitForCssVars();
    expect(rafQueue.size).toBe(0);
  });

  it('never re-enters init() once disconnected', () => {
    const gradient = mountedGradient();
    const initMesh = vi.spyOn(gradient, 'initMesh');

    gradient.disconnect();
    gradient.init();

    expect(initMesh).not.toHaveBeenCalled();
  });

  it("cancels init()'s opening animation frame on disconnect()", () => {
    const gradient = mountedGradient();
    // Stub the WebGL setup so init() reaches its requestAnimationFrame call
    // without a real GL context. init() swallows throws, so the size assertion
    // below is what proves the frame was actually scheduled.
    vi.spyOn(gradient, 'initGradientColors').mockImplementation(() => {});
    vi.spyOn(gradient, 'initMesh').mockImplementation(() => {});
    vi.spyOn(gradient, 'resize').mockImplementation(() => {});

    gradient.init();
    expect(rafQueue.size).toBe(1);
    const openingFrame = nextRafHandle;

    gradient.disconnect();

    // Pre-fix this handle was never stored on `animateRaf`, so disconnect() had
    // nothing to cancel and the opening frame outlived teardown.
    expect(cancelled).toContain(openingFrame);
    expect(rafQueue.size).toBe(0);
  });

  it('drops an animation frame that lands after disconnect() instead of throwing', () => {
    const gradient = mountedGradient();
    gradient.conf = { playing: true };

    gradient.play();
    expect(rafQueue.size).toBe(1);
    const queuedFrame = [...rafQueue.values()][0];

    gradient.disconnect();

    // `mesh` never existed here, so the pre-fix animate() would have thrown on
    // `this.mesh.material` for this already-queued frame.
    expect(() => queuedFrame(performance.now())).not.toThrow();
  });

  it('cancels the queued animation frame on pause() and refuses play() after disconnect()', () => {
    const gradient = mountedGradient();
    const conf = { playing: false };
    gradient.conf = conf;

    gradient.play();
    expect(conf.playing).toBe(true);
    const handle = nextRafHandle;

    gradient.pause();
    expect(cancelled).toContain(handle);
    expect(conf.playing).toBe(false);

    gradient.disconnect();
    gradient.play();
    expect(rafQueue.size).toBe(0);
    expect(conf.playing).toBe(false);
  });

  it('ignores a resize event fired after teardown', () => {
    const gradient = mountedGradient();

    gradient.disconnect();
    // `minigl`/`mesh` are absent, so the pre-fix handler threw here.
    expect(() => gradient.resize()).not.toThrow();
  });

  it('is safe to disconnect twice', () => {
    const gradient = mountedGradient();

    gradient.addIsLoadedClass();
    gradient.disconnect();
    expect(() => gradient.disconnect()).not.toThrow();

    flushFrame();
    vi.advanceTimersByTime(3000);
    expect(wrapper.classList.contains('isLoaded')).toBe(false);
  });
});
