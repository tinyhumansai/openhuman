import { act, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import RotatingTetrahedronCanvas from './RotatingTetrahedronCanvas';

const motion = vi.hoisted(() => ({ allowed: true }));
const renderer = vi.hoisted(() => ({
  render: vi.fn(),
  setSize: vi.fn(),
  setPixelRatio: vi.fn(),
  dispose: vi.fn(),
  failConstruction: false,
}));

vi.mock('../hooks/useAnimationActivity', () => ({ useAnimationActivity: () => motion.allowed }));

vi.mock('../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

vi.mock('three', async importOriginal => {
  const actual = await importOriginal<typeof import('three')>();
  return {
    ...actual,
    WebGLRenderer: class {
      constructor() {
        if (renderer.failConstruction) throw new Error('WebGL renderer unavailable');
      }
      render = renderer.render;
      setSize = renderer.setSize;
      setPixelRatio = renderer.setPixelRatio;
      dispose = renderer.dispose;
      outputColorSpace = actual.SRGBColorSpace;
      toneMapping = actual.NoToneMapping;
    },
  };
});

class MockResizeObserver {
  observe = vi.fn();
  disconnect = vi.fn();
}

describe('RotatingTetrahedronCanvas animation lifecycle', () => {
  const originalGetContext = HTMLCanvasElement.prototype.getContext;
  let frames: FrameRequestCallback[];
  let requestFrame: ReturnType<typeof vi.fn>;
  let cancelFrame: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.clearAllMocks();
    motion.allowed = true;
    renderer.failConstruction = false;
    frames = [];
    requestFrame = vi.fn((callback: FrameRequestCallback) => {
      frames.push(callback);
      return frames.length;
    });
    cancelFrame = vi.fn();
    vi.stubGlobal('requestAnimationFrame', requestFrame);
    vi.stubGlobal('cancelAnimationFrame', cancelFrame);
    vi.stubGlobal('ResizeObserver', MockResizeObserver);
    Object.defineProperty(HTMLCanvasElement.prototype, 'getContext', {
      configurable: true,
      value: vi.fn(() => ({})),
    });
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({
      width: 120,
      height: 90,
    } as DOMRect);
  });

  afterEach(() => {
    Object.defineProperty(HTMLCanvasElement.prototype, 'getContext', {
      configurable: true,
      value: originalGetContext,
    });
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('retains a static frame and no animation loop when decorative motion is disabled', () => {
    motion.allowed = false;
    const { container, unmount } = render(<RotatingTetrahedronCanvas />);
    expect(container.querySelector('canvas')).not.toBeNull();
    expect(renderer.render).toHaveBeenCalled();
    expect(renderer.setSize).toHaveBeenCalledWith(120, 90, false);
    expect(requestFrame).not.toHaveBeenCalled();
    unmount();
    expect(renderer.dispose).toHaveBeenCalled();
  });

  it('pauses and resumes a single animation loop without recreating the scene', () => {
    const { rerender, unmount } = render(<RotatingTetrahedronCanvas />);
    expect(requestFrame).toHaveBeenCalledTimes(1);

    act(() => {
      frames[0]?.(100);
    });
    expect(renderer.render).toHaveBeenCalled();
    expect(requestFrame).toHaveBeenCalledTimes(2);

    motion.allowed = false;
    rerender(<RotatingTetrahedronCanvas />);
    expect(cancelFrame).toHaveBeenCalledWith(2);
    expect(requestFrame).toHaveBeenCalledTimes(2);

    motion.allowed = true;
    rerender(<RotatingTetrahedronCanvas inverted />);
    expect(requestFrame).toHaveBeenCalledTimes(3);
    expect(renderer.render).toHaveBeenCalled();
    unmount();
    expect(cancelFrame).toHaveBeenCalledWith(3);
    expect(renderer.dispose).toHaveBeenCalledTimes(1);
  });

  it('renders no canvas when a WebGL context is unavailable', () => {
    Object.defineProperty(HTMLCanvasElement.prototype, 'getContext', {
      configurable: true,
      value: vi.fn(() => null),
    });
    const { container } = render(<RotatingTetrahedronCanvas />);
    expect(container.querySelector('canvas')).toBeNull();
    expect(requestFrame).not.toHaveBeenCalled();
  });

  it('handles a failed Three.js renderer without scheduling frames', () => {
    renderer.failConstruction = true;
    const { container } = render(<RotatingTetrahedronCanvas />);
    expect(container.querySelector('canvas')).toBeNull();
    expect(requestFrame).not.toHaveBeenCalled();
  });
});
