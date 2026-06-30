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
