/**
 * Tests for the native directory chooser wrapper (#5831).
 *
 * The point of every case here is the same: a caller must be able to tell
 * "the user chose nothing" from "no path could be obtained", because only
 * the second one is allowed to surface an error, and neither is allowed to
 * produce a path the caller then stores.
 */
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { isTauri, safeInvoke } from './common';
import { pickDirectoryNatively } from './directoryPicker';

vi.mock('./common', () => ({ isTauri: vi.fn(), safeInvoke: vi.fn() }));

describe('tauriCommands/directoryPicker', () => {
  beforeEach(() => {
    vi.mocked(safeInvoke).mockReset();
    vi.mocked(isTauri).mockReset();
    vi.mocked(isTauri).mockReturnValue(true);
  });

  test('reports unavailable without invoking when there is no host to ask', async () => {
    vi.mocked(isTauri).mockReturnValue(false);

    await expect(pickDirectoryNatively()).resolves.toEqual({ ok: false, reason: 'unavailable' });

    expect(safeInvoke).not.toHaveBeenCalled();
  });

  test('returns the absolute path the host chooser produced', async () => {
    vi.mocked(safeInvoke).mockResolvedValue('/Users/you/notes');

    await expect(pickDirectoryNatively()).resolves.toEqual({ ok: true, path: '/Users/you/notes' });

    expect(safeInvoke).toHaveBeenCalledWith('pick_directory_via_dialog');
  });

  test('treats a null result as a user cancellation, not a failure', async () => {
    vi.mocked(safeInvoke).mockResolvedValue(null);

    await expect(pickDirectoryNatively()).resolves.toEqual({ ok: false, reason: 'cancelled' });
  });

  test('treats an empty string as a cancellation rather than an empty path', async () => {
    vi.mocked(safeInvoke).mockResolvedValue('');

    await expect(pickDirectoryNatively()).resolves.toEqual({ ok: false, reason: 'cancelled' });
  });

  test('converts an invoke rejection into a failed result instead of throwing', async () => {
    vi.mocked(safeInvoke).mockRejectedValue(new Error('no chooser on this host'));

    await expect(pickDirectoryNatively()).resolves.toEqual({
      ok: false,
      reason: 'failed',
      message: 'no chooser on this host',
    });
  });

  test('carries a non-Error rejection through as a string message', async () => {
    vi.mocked(safeInvoke).mockRejectedValue('portal timed out');

    await expect(pickDirectoryNatively()).resolves.toEqual({
      ok: false,
      reason: 'failed',
      message: 'portal timed out',
    });
  });
});
