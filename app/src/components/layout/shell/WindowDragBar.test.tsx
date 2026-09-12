import { cleanup, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import WindowDragBar, { WINDOW_DRAG_BAR_HEIGHT } from './WindowDragBar';

// Both gates are mocked so we can drive every platform combination.
const isMac = vi.fn();
const isTauri = vi.fn();
vi.mock('../../../lib/commands/shortcut', () => ({ isMac: () => isMac() }));
vi.mock('../../../utils/tauriCommands/common', () => ({ isTauri: () => isTauri() }));

describe('WindowDragBar', () => {
  beforeEach(() => {
    isMac.mockReset();
    isTauri.mockReset();
  });
  afterEach(cleanup);

  it('renders a draggable strip on macOS inside Tauri', () => {
    isMac.mockReturnValue(true);
    isTauri.mockReturnValue(true);
    const { container } = render(<WindowDragBar />);
    const bar = container.querySelector('[data-tauri-drag-region]');
    expect(bar).not.toBeNull();
    expect((bar as HTMLElement).style.height).toBe(`${WINDOW_DRAG_BAR_HEIGHT}px`);
    expect((bar as HTMLElement).className).toContain('absolute');
  });

  // Presence is what the assertions above cover, and presence is exactly what a
  // regression keeps: `drag.js` drags a bare region only on a direct hit
  // (`el === composedPath[0]`), so the attribute's *value* is the behaviour.
  // Bare is right for this band only because it has no children — assert both
  // halves, so giving it a child without switching to `"deep"` fails here
  // rather than silently killing the drag the way it did in the sidebar.
  it('marks the band bare, which is only correct because it has no children', () => {
    isMac.mockReturnValue(true);
    isTauri.mockReturnValue(true);
    const { container } = render(<WindowDragBar />);
    const bar = container.querySelector('[data-tauri-drag-region]') as HTMLElement;
    expect(bar.getAttribute('data-tauri-drag-region')).toBe('true');
    expect(bar.children).toHaveLength(0);
  });

  it('renders nothing on macOS outside Tauri (plain browser)', () => {
    isMac.mockReturnValue(true);
    isTauri.mockReturnValue(false);
    const { container } = render(<WindowDragBar />);
    expect(container.querySelector('[data-tauri-drag-region]')).toBeNull();
  });

  it('renders nothing inside Tauri on non-macOS (native title bar handles drag)', () => {
    isMac.mockReturnValue(false);
    isTauri.mockReturnValue(true);
    const { container } = render(<WindowDragBar />);
    expect(container.querySelector('[data-tauri-drag-region]')).toBeNull();
  });
});
