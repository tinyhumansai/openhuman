import { act } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import ThemeProvider from '../providers/ThemeProvider';
import { setThemeToken, upsertCustomTheme } from '../store/themeSlice';
import { renderWithProviders } from '../test/test-utils';
import MeshGradient from './MeshGradient';

const gradientMock = vi.hoisted(() => {
  // Shared play flag, mirroring the real `Gradient.conf.playing`, so the
  // component's double-schedule guard (`shouldAnimate === playing`) is exercised.
  const conf = { playing: false };
  // `state.mesh` mirrors `Gradient.mesh`: truthy once a WebGL context is
  // acquired, `undefined` on no-GPU/headless. The component only calls play()
  // when it is set (#3524); a getter lets a test flip it before events fire.
  const state: { mesh: unknown } = { mesh: {} };
  return {
    conf,
    state,
    disconnect: vi.fn(),
    initGradient: vi.fn(),
    pause: vi.fn(() => {
      conf.playing = false;
    }),
    play: vi.fn(() => {
      conf.playing = true;
    }),
    // eslint-disable-next-line prefer-arrow-callback -- constructor mock must be new-able; arrows are not constructible.
    Gradient: vi.fn(function MockGradient() {
      return {
        conf,
        get mesh() {
          return state.mesh;
        },
        disconnect: gradientMock.disconnect,
        initGradient: gradientMock.initGradient,
        pause: gradientMock.pause,
        play: gradientMock.play,
      };
    }),
  };
});

vi.mock('../lib/meshGradient', () => ({ Gradient: gradientMock.Gradient }));

describe('<MeshGradient />', () => {
  let rafQueue: FrameRequestCallback[];

  beforeEach(() => {
    gradientMock.disconnect.mockClear();
    gradientMock.Gradient.mockClear();
    gradientMock.initGradient.mockClear();
    gradientMock.pause.mockClear();
    gradientMock.play.mockClear();
    gradientMock.conf.playing = false;
    gradientMock.state.mesh = {}; // default: WebGL mesh initialized OK
    // Default to a visible, focused window so the gradient animates unless a
    // test says otherwise.
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    rafQueue = [];
    vi.spyOn(window, 'requestAnimationFrame').mockImplementation(callback => {
      rafQueue.push(callback);
      return rafQueue.length;
    });
    vi.spyOn(window, 'cancelAnimationFrame').mockImplementation(() => {});
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  function flushAnimationFrames() {
    const pending = [...rafQueue];
    rafQueue = [];
    for (const callback of pending) {
      callback(performance.now());
    }
  }

  it('restarts when a custom theme mesh colour changes without changing theme id', () => {
    const { container, store } = renderWithProviders(
      <ThemeProvider>
        <MeshGradient />
      </ThemeProvider>
    );

    act(() => {
      store.dispatch(
        upsertCustomTheme({
          id: 'custom-live',
          name: 'Live',
          isDark: false,
          builtIn: false,
          colors: {
            'primary-700': '1 2 3',
            'primary-300': '4 5 6',
            'primary-500': '7 8 9',
            surface: '10 11 12',
          },
          fonts: {},
        })
      );
    });
    act(() => {
      flushAnimationFrames();
    });

    const canvas = container.querySelector('#mesh-gradient') as HTMLCanvasElement;
    expect(canvas.style.getPropertyValue('--gradient-color-4')).toBe('#070809');
    expect(gradientMock.initGradient).toHaveBeenCalledTimes(1);

    act(() => {
      store.dispatch(setThemeToken({ key: 'primary-500', value: '20 30 40' }));
    });
    act(() => {
      flushAnimationFrames();
    });

    expect(canvas.style.getPropertyValue('--gradient-color-4')).toBe('#141e28');
    expect(gradientMock.disconnect).toHaveBeenCalledTimes(1);
    expect(gradientMock.pause).toHaveBeenCalledTimes(1);
    expect(gradientMock.initGradient).toHaveBeenCalledTimes(2);
  });

  it('pauses the animation when the window loses focus and resumes when it returns (#3524)', () => {
    const hasFocus = vi.spyOn(document, 'hasFocus').mockReturnValue(true);

    renderWithProviders(
      <ThemeProvider>
        <MeshGradient />
      </ThemeProvider>
    );
    act(() => {
      flushAnimationFrames();
    });

    // Focused + visible on mount → animating.
    expect(gradientMock.play).toHaveBeenCalledTimes(1);
    expect(gradientMock.conf.playing).toBe(true);
    gradientMock.play.mockClear();
    gradientMock.pause.mockClear();

    // Window backgrounded (occluded/blurred) → the shader must stop rendering.
    hasFocus.mockReturnValue(false);
    act(() => {
      window.dispatchEvent(new Event('blur'));
    });
    expect(gradientMock.pause).toHaveBeenCalledTimes(1);
    expect(gradientMock.conf.playing).toBe(false);

    // Window refocused → resume.
    hasFocus.mockReturnValue(true);
    act(() => {
      window.dispatchEvent(new Event('focus'));
    });
    expect(gradientMock.play).toHaveBeenCalledTimes(1);
    expect(gradientMock.conf.playing).toBe(true);
  });

  it('never resumes when the WebGL mesh failed to initialize (no-GPU/Tauri, #3524)', () => {
    // Simulate a gradient whose connect() couldn't get a GL context: no `mesh`
    // was ever built, yet the real lib leaves `conf.playing` truthy. play() must
    // stay suppressed so the animation loop never dereferences the missing mesh.
    gradientMock.state.mesh = undefined;
    gradientMock.conf.playing = true;

    const hasFocus = vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    renderWithProviders(
      <ThemeProvider>
        <MeshGradient />
      </ThemeProvider>
    );
    act(() => {
      flushAnimationFrames();
    });

    // Blur then refocus — the resume path must NOT call play() without a mesh
    // (previously this crashed on the next animation frame).
    hasFocus.mockReturnValue(false);
    act(() => {
      window.dispatchEvent(new Event('blur'));
    });
    hasFocus.mockReturnValue(true);
    act(() => {
      window.dispatchEvent(new Event('focus'));
    });

    expect(gradientMock.play).not.toHaveBeenCalled();
  });
});
