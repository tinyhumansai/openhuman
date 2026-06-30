import { act } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import ThemeProvider from '../providers/ThemeProvider';
import { setThemeToken, upsertCustomTheme } from '../store/themeSlice';
import { renderWithProviders } from '../test/test-utils';
import MeshGradient from './MeshGradient';

const gradientMock = vi.hoisted(() => ({
  disconnect: vi.fn(),
  // eslint-disable-next-line prefer-arrow-callback -- constructor mock must be new-able; arrows are not constructible.
  Gradient: vi.fn(function MockGradient() {
    return {
      disconnect: gradientMock.disconnect,
      initGradient: gradientMock.initGradient,
      pause: gradientMock.pause,
    };
  }),
  initGradient: vi.fn(),
  pause: vi.fn(),
}));

vi.mock('../lib/meshGradient', () => ({ Gradient: gradientMock.Gradient }));

describe('<MeshGradient />', () => {
  let rafQueue: FrameRequestCallback[];

  beforeEach(() => {
    gradientMock.disconnect.mockClear();
    gradientMock.Gradient.mockClear();
    gradientMock.initGradient.mockClear();
    gradientMock.pause.mockClear();
    rafQueue = [];
    vi.spyOn(window, 'requestAnimationFrame').mockImplementation(callback => {
      rafQueue.push(callback);
      return rafQueue.length;
    });
    vi.spyOn(window, 'cancelAnimationFrame').mockImplementation(() => {});
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
});
