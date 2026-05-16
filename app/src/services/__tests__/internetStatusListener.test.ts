/**
 * Tests for internetStatusListener — covers changed lines 12, 14-16, 19-22, 24-26.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// Mock the store before importing the module under test.
const dispatchMock = vi.fn();
vi.mock('../../store/index', () => ({ store: { dispatch: dispatchMock } }));

// Capture what gets dispatched without caring about the action creator shape.
const setInternetMock = vi.fn((payload: unknown) => ({
  type: 'connectivity/setInternet',
  payload,
}));
vi.mock('../../store/connectivitySlice', () => ({
  setInternet: (p: unknown) => setInternetMock(p),
}));

describe('internetStatusListener', () => {
  // Each test needs a fresh module so the `started` singleton is reset.
  beforeEach(() => {
    vi.resetModules();
    dispatchMock.mockClear();
    setInternetMock.mockClear();
  });

  afterEach(() => {
    // Remove any event listeners that were added during the test.
    window.removeEventListener('online', () => {});
    window.removeEventListener('offline', () => {});
  });

  it('dispatches online when navigator.onLine is true on start (line 15-16)', async () => {
    Object.defineProperty(navigator, 'onLine', { value: true, configurable: true });

    const { startInternetStatusListener } = await import('../internetStatusListener');
    startInternetStatusListener();

    expect(setInternetMock).toHaveBeenCalledWith({ value: 'online' });
    expect(dispatchMock).toHaveBeenCalled();
  });

  it('dispatches offline when navigator.onLine is false on start (line 15-16)', async () => {
    Object.defineProperty(navigator, 'onLine', { value: false, configurable: true });

    const { startInternetStatusListener } = await import('../internetStatusListener');
    startInternetStatusListener();

    expect(setInternetMock).toHaveBeenCalledWith({ value: 'offline' });
    expect(dispatchMock).toHaveBeenCalled();
  });

  it('is idempotent — second call is a no-op (line 20)', async () => {
    Object.defineProperty(navigator, 'onLine', { value: true, configurable: true });

    const { startInternetStatusListener } = await import('../internetStatusListener');
    startInternetStatusListener();
    startInternetStatusListener(); // second call must not add extra listeners or dispatch

    // dispatch only called once for the initial snapshot
    expect(dispatchMock).toHaveBeenCalledTimes(1);
  });

  it('dispatches online when the window online event fires (lines 24-26)', async () => {
    Object.defineProperty(navigator, 'onLine', { value: false, configurable: true });

    const { startInternetStatusListener } = await import('../internetStatusListener');
    startInternetStatusListener();

    dispatchMock.mockClear();
    setInternetMock.mockClear();

    // Simulate navigator going online before firing the event.
    Object.defineProperty(navigator, 'onLine', { value: true, configurable: true });
    window.dispatchEvent(new Event('online'));

    expect(setInternetMock).toHaveBeenCalledWith({ value: 'online' });
    expect(dispatchMock).toHaveBeenCalled();
  });

  it('dispatches offline when the window offline event fires (lines 24-26)', async () => {
    Object.defineProperty(navigator, 'onLine', { value: true, configurable: true });

    const { startInternetStatusListener } = await import('../internetStatusListener');
    startInternetStatusListener();

    dispatchMock.mockClear();
    setInternetMock.mockClear();

    Object.defineProperty(navigator, 'onLine', { value: false, configurable: true });
    window.dispatchEvent(new Event('offline'));

    expect(setInternetMock).toHaveBeenCalledWith({ value: 'offline' });
    expect(dispatchMock).toHaveBeenCalled();
  });
});
