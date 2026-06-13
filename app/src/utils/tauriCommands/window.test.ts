import { invoke } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { isTauri } from './common';
import { notchWindowHide, notchWindowShow, syncNotchVisibility } from './window';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('./common', () => ({ isTauri: vi.fn() }));

describe('tauriCommands/window — notch indicator', () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
    vi.mocked(isTauri).mockReset();
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(invoke).mockResolvedValue(undefined);
  });

  test('notchWindowShow invokes the notch_window_show command', async () => {
    await notchWindowShow();
    expect(invoke).toHaveBeenCalledWith('notch_window_show');
  });

  test('notchWindowHide invokes the notch_window_hide command', async () => {
    await notchWindowHide();
    expect(invoke).toHaveBeenCalledWith('notch_window_hide');
  });

  test('notch commands no-op (and never invoke) outside Tauri', async () => {
    vi.mocked(isTauri).mockReturnValue(false);

    await notchWindowShow();
    await notchWindowHide();

    expect(invoke).not.toHaveBeenCalled();
  });

  test('notch commands swallow invoke rejections so callers never throw', async () => {
    vi.mocked(invoke).mockRejectedValue(new Error('panel build failed'));

    // Must resolve, not reject — notch visibility is cosmetic and should never
    // surface as an error to the caller.
    await expect(notchWindowShow()).resolves.toBeUndefined();
    await expect(notchWindowHide()).resolves.toBeUndefined();
  });

  test('syncNotchVisibility(true) shows the notch', async () => {
    await syncNotchVisibility(true);
    expect(invoke).toHaveBeenCalledWith('notch_window_show');
    expect(invoke).not.toHaveBeenCalledWith('notch_window_hide');
  });

  test('syncNotchVisibility(false) hides the notch', async () => {
    await syncNotchVisibility(false);
    expect(invoke).toHaveBeenCalledWith('notch_window_hide');
    expect(invoke).not.toHaveBeenCalledWith('notch_window_show');
  });
});
