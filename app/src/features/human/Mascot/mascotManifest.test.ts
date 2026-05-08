import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  __resetMascotManifestForTests,
  isMascotColorAvailable,
  loadMascotManifest,
  setAvailableMascotColors,
} from './mascotManifest';

describe('mascotManifest', () => {
  beforeEach(() => {
    __resetMascotManifestForTests();
  });

  afterEach(() => {
    vi.restoreAllMocks();
    __resetMascotManifestForTests();
  });

  describe('isMascotColorAvailable', () => {
    it('defaults to yellow-only availability', () => {
      expect(isMascotColorAvailable('yellow')).toBe(true);
      expect(isMascotColorAvailable('navy')).toBe(false);
      expect(isMascotColorAvailable('burgundy')).toBe(false);
    });
  });

  describe('setAvailableMascotColors', () => {
    it('replaces the available set with the provided colors', () => {
      setAvailableMascotColors(['yellow', 'burgundy', 'navy']);
      expect(isMascotColorAvailable('burgundy')).toBe(true);
      expect(isMascotColorAvailable('navy')).toBe(true);
      expect(isMascotColorAvailable('green')).toBe(false);
    });

    it('always keeps yellow available even when omitted from the input', () => {
      setAvailableMascotColors(['navy']);
      expect(isMascotColorAvailable('yellow')).toBe(true);
      expect(isMascotColorAvailable('navy')).toBe(true);
    });
  });

  describe('loadMascotManifest', () => {
    it('expands availability based on the fetched manifest', async () => {
      const fetchMock = vi
        .fn()
        .mockResolvedValue({
          ok: true,
          json: async () => ({
            variants: [
              { color: 'yellow' },
              { color: 'burgundy' },
              { color: 'navy' },
              { color: 'unknown-color' },
            ],
          }),
        });
      window.fetch = fetchMock as unknown as typeof fetch;

      await loadMascotManifest();

      expect(fetchMock).toHaveBeenCalledOnce();
      expect(isMascotColorAvailable('yellow')).toBe(true);
      expect(isMascotColorAvailable('burgundy')).toBe(true);
      expect(isMascotColorAvailable('navy')).toBe(true);
      expect(isMascotColorAvailable('green')).toBe(false);
    });

    it('memoizes the load promise so repeat calls do not refetch', async () => {
      const fetchMock = vi
        .fn()
        .mockResolvedValue({ ok: true, json: async () => ({ variants: [{ color: 'yellow' }] }) });
      window.fetch = fetchMock as unknown as typeof fetch;

      await loadMascotManifest();
      await loadMascotManifest();

      expect(fetchMock).toHaveBeenCalledOnce();
    });

    it('keeps the default availability when the fetch responds non-ok', async () => {
      const fetchMock = vi.fn().mockResolvedValue({ ok: false, json: async () => ({}) });
      window.fetch = fetchMock as unknown as typeof fetch;

      await loadMascotManifest();

      expect(isMascotColorAvailable('yellow')).toBe(true);
      expect(isMascotColorAvailable('navy')).toBe(false);
    });

    it('swallows fetch errors and keeps yellow-only availability', async () => {
      window.fetch = vi.fn().mockRejectedValue(new Error('offline')) as unknown as typeof fetch;

      await loadMascotManifest();

      expect(isMascotColorAvailable('yellow')).toBe(true);
      expect(isMascotColorAvailable('burgundy')).toBe(false);
    });

    it('ignores manifests with no usable variants', async () => {
      window.fetch = vi
        .fn()
        .mockResolvedValue({
          ok: true,
          json: async () => ({ variants: [] }),
        }) as unknown as typeof fetch;

      await loadMascotManifest();

      expect(isMascotColorAvailable('yellow')).toBe(true);
      expect(isMascotColorAvailable('navy')).toBe(false);
    });
  });
});
