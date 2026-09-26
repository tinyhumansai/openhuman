import { fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import WindowsWindowControls from './WindowsWindowControls';

const { minimizeWindow, maximizeWindow, quitApp } = vi.hoisted(() => ({
  minimizeWindow: vi.fn(),
  maximizeWindow: vi.fn(),
  quitApp: vi.fn(),
}));

vi.mock('../../../utils/tauriCommands/common', () => ({ isTauri: () => true }));
vi.mock('../../../utils/tauriCommands/window', () => ({ minimizeWindow, maximizeWindow, quitApp }));
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    isMaximized: () => Promise.resolve(false),
    onResized: () => Promise.resolve(() => {}),
  }),
}));

describe('WindowsWindowControls', () => {
  const platform = navigator.platform;

  beforeEach(() => {
    Object.defineProperty(navigator, 'platform', { configurable: true, value: 'Win32' });
    vi.clearAllMocks();
  });

  afterEach(() => {
    Object.defineProperty(navigator, 'platform', { configurable: true, value: platform });
  });

  it('uses the full app quit path for the close button', () => {
    render(<WindowsWindowControls />);
    fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(quitApp).toHaveBeenCalledTimes(1);
  });
});
