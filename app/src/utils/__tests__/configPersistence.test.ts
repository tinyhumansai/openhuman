/**
 * Unit tests for configPersistence utilities.
 * Tests URL storage, validation, and normalization.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  clearStoredCoreMode,
  clearStoredCoreToken,
  clearStoredRpcUrl,
  getDefaultRpcUrl,
  getStoredCoreMode,
  getStoredCoreToken,
  getStoredRpcUrl,
  isValidRpcUrl,
  normalizeRpcUrl,
  peekStoredRpcUrl,
  storeCoreMode,
  storeCoreToken,
  storeRpcUrl,
} from '../configPersistence';

const STORAGE_KEY = 'openhuman_core_rpc_url';
const TOKEN_STORAGE_KEY = 'openhuman_core_rpc_token';
const MODE_STORAGE_KEY = 'openhuman_core_mode';

describe('configPersistence', () => {
  beforeEach(() => {
    // Clear localStorage before each test
    localStorage.removeItem(STORAGE_KEY);
    localStorage.removeItem(TOKEN_STORAGE_KEY);
    localStorage.removeItem(MODE_STORAGE_KEY);
  });

  afterEach(() => {
    // Clean up after each test
    localStorage.removeItem(STORAGE_KEY);
    localStorage.removeItem(TOKEN_STORAGE_KEY);
    localStorage.removeItem(MODE_STORAGE_KEY);
  });

  describe('getStoredRpcUrl', () => {
    it('returns default URL when no URL is stored', () => {
      const result = getStoredRpcUrl();
      expect(result).toBe('http://127.0.0.1:7788/rpc');
    });

    it('returns stored URL when available', () => {
      localStorage.setItem(STORAGE_KEY, 'http://localhost:8080/rpc');
      const result = getStoredRpcUrl();
      expect(result).toBe('http://localhost:8080/rpc');
    });

    it('trims whitespace from stored URL', () => {
      localStorage.setItem(STORAGE_KEY, '  http://localhost:8080/rpc  ');
      const result = getStoredRpcUrl();
      expect(result).toBe('http://localhost:8080/rpc');
    });

    it('returns default when stored URL is empty', () => {
      localStorage.setItem(STORAGE_KEY, '');
      const result = getStoredRpcUrl();
      expect(result).toBe('http://127.0.0.1:7788/rpc');
    });
  });

  describe('storeRpcUrl', () => {
    it('stores a valid URL', () => {
      storeRpcUrl('http://localhost:9000/rpc');
      expect(localStorage.getItem(STORAGE_KEY)).toBe('http://localhost:9000/rpc');
    });

    it('trims and stores URL', () => {
      storeRpcUrl('  http://localhost:9000/rpc  ');
      expect(localStorage.getItem(STORAGE_KEY)).toBe('http://localhost:9000/rpc');
    });

    it('clears stored URL when given empty string', () => {
      localStorage.setItem(STORAGE_KEY, 'http://localhost:9000/rpc');
      storeRpcUrl('');
      expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
    });

    it('clears stored URL when given whitespace-only string', () => {
      localStorage.setItem(STORAGE_KEY, 'http://localhost:9000/rpc');
      storeRpcUrl('   ');
      expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
    });
  });

  describe('clearStoredRpcUrl', () => {
    it('removes stored URL', () => {
      localStorage.setItem(STORAGE_KEY, 'http://localhost:9000/rpc');
      clearStoredRpcUrl();
      expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
    });
  });

  describe('isValidRpcUrl', () => {
    it('returns true for valid http URL', () => {
      expect(isValidRpcUrl('http://localhost:7788/rpc')).toBe(true);
    });

    it('returns true for valid https URL', () => {
      expect(isValidRpcUrl('https://api.example.com/rpc')).toBe(true);
    });

    it('returns true for URL without /rpc suffix', () => {
      expect(isValidRpcUrl('http://localhost:7788')).toBe(true);
    });

    it('returns false for empty string', () => {
      expect(isValidRpcUrl('')).toBe(false);
    });

    it('returns false for whitespace-only string', () => {
      expect(isValidRpcUrl('   ')).toBe(false);
    });

    it('returns false for null/undefined', () => {
      expect(isValidRpcUrl(null as unknown as string)).toBe(false);
      expect(isValidRpcUrl(undefined as unknown as string)).toBe(false);
    });

    it('returns false for invalid protocol', () => {
      expect(isValidRpcUrl('ftp://localhost:7788/rpc')).toBe(false);
      expect(isValidRpcUrl('ws://localhost:7788/rpc')).toBe(false);
    });

    it('returns false for malformed URL', () => {
      expect(isValidRpcUrl('not a valid url')).toBe(false);
      expect(isValidRpcUrl('http://')).toBe(false);
    });
  });

  describe('normalizeRpcUrl', () => {
    it('trims whitespace', () => {
      expect(normalizeRpcUrl('  http://localhost:7788/rpc  ')).toBe('http://localhost:7788/rpc');
    });

    it('removes trailing slashes', () => {
      expect(normalizeRpcUrl('http://localhost:7788/rpc/')).toBe('http://localhost:7788/rpc');
      expect(normalizeRpcUrl('http://localhost:7788/')).toBe('http://localhost:7788');
    });

    it('handles multiple trailing slashes', () => {
      expect(normalizeRpcUrl('http://localhost:7788/rpc///')).toBe('http://localhost:7788/rpc');
    });

    it('preserves URL without trailing slash', () => {
      expect(normalizeRpcUrl('http://localhost:7788/rpc')).toBe('http://localhost:7788/rpc');
    });
  });

  describe('getDefaultRpcUrl', () => {
    it('returns the expected default URL', () => {
      expect(getDefaultRpcUrl()).toBe('http://127.0.0.1:7788/rpc');
    });
  });

  describe('isValidRpcUrl — edge cases', () => {
    it('returns true for localhost with a port', () => {
      expect(isValidRpcUrl('http://localhost:7788')).toBe(true);
    });

    it('returns true for a bare IP address URL', () => {
      expect(isValidRpcUrl('http://192.168.1.100:7788/rpc')).toBe(true);
    });

    it('returns true for an HTTPS URL', () => {
      expect(isValidRpcUrl('https://remote-core.example.com/rpc')).toBe(true);
    });

    it('returns true for a URL with a path segment', () => {
      expect(isValidRpcUrl('http://127.0.0.1:7788/rpc')).toBe(true);
    });

    it('returns false for empty string', () => {
      expect(isValidRpcUrl('')).toBe(false);
    });

    it('returns false for whitespace-only string', () => {
      expect(isValidRpcUrl('   ')).toBe(false);
    });

    it('returns false for a URL without a protocol', () => {
      expect(isValidRpcUrl('localhost:7788/rpc')).toBe(false);
      expect(isValidRpcUrl('127.0.0.1:7788')).toBe(false);
    });

    it('returns false for a ws:// URL', () => {
      expect(isValidRpcUrl('ws://localhost:7788')).toBe(false);
    });

    it('returns false for a ftp:// URL', () => {
      expect(isValidRpcUrl('ftp://localhost:7788')).toBe(false);
    });

    it('returns false for a completely malformed string', () => {
      expect(isValidRpcUrl('not a url at all')).toBe(false);
    });

    it('returns false for http:// with no host', () => {
      expect(isValidRpcUrl('http://')).toBe(false);
    });
  });

  describe('normalizeRpcUrl — edge cases', () => {
    it('does not add /rpc suffix when missing (normalizeRpcUrl only strips, not appends)', () => {
      expect(normalizeRpcUrl('http://127.0.0.1:7788')).toBe('http://127.0.0.1:7788');
    });

    it('does not double-add /rpc — leaves existing /rpc alone', () => {
      expect(normalizeRpcUrl('http://127.0.0.1:7788/rpc')).toBe('http://127.0.0.1:7788/rpc');
    });

    it('handles trailing slash after /rpc', () => {
      expect(normalizeRpcUrl('http://127.0.0.1:7788/rpc/')).toBe('http://127.0.0.1:7788/rpc');
    });

    it('handles uppercase protocol casing (trims only, does not lowercase)', () => {
      // The normalizer does not lowercase — it just trims slashes and whitespace
      expect(normalizeRpcUrl('  HTTP://localhost:7788/rpc  ')).toBe('HTTP://localhost:7788/rpc');
    });

    it('removes multiple trailing slashes', () => {
      expect(normalizeRpcUrl('http://127.0.0.1:7788/rpc///')).toBe('http://127.0.0.1:7788/rpc');
    });

    it('trims leading and trailing whitespace', () => {
      expect(normalizeRpcUrl('  http://127.0.0.1:7788/rpc  ')).toBe('http://127.0.0.1:7788/rpc');
    });
  });

  describe('storeRpcUrl + getStoredRpcUrl — round-trip', () => {
    it('round-trips an HTTPS URL', () => {
      storeRpcUrl('https://remote.example.com/rpc');
      expect(getStoredRpcUrl()).toBe('https://remote.example.com/rpc');
    });

    it('round-trips a localhost URL with a non-standard port', () => {
      storeRpcUrl('http://localhost:12345/rpc');
      expect(getStoredRpcUrl()).toBe('http://localhost:12345/rpc');
    });

    it('round-trips an IP address URL', () => {
      storeRpcUrl('http://10.0.0.1:7788/rpc');
      expect(getStoredRpcUrl()).toBe('http://10.0.0.1:7788/rpc');
    });
  });

  describe('clearStoredRpcUrl + getStoredRpcUrl', () => {
    it('getStoredRpcUrl returns the default after clearStoredRpcUrl', () => {
      storeRpcUrl('http://some-host:9999/rpc');
      expect(getStoredRpcUrl()).toBe('http://some-host:9999/rpc');

      clearStoredRpcUrl();
      expect(getStoredRpcUrl()).toBe('http://127.0.0.1:7788/rpc');
    });

    it('localStorage key is null after clearStoredRpcUrl', () => {
      storeRpcUrl('http://some-host:9999/rpc');
      clearStoredRpcUrl();
      expect(localStorage.getItem('openhuman_core_rpc_url')).toBeNull();
    });
  });

  describe('getStoredRpcUrl — localStorage unavailable', () => {
    it('returns the default URL when localStorage throws', () => {
      const getItemSpy = vi.spyOn(localStorage, 'getItem').mockImplementation(() => {
        throw new Error('Storage unavailable');
      });
      try {
        expect(getStoredRpcUrl()).toBe('http://127.0.0.1:7788/rpc');
      } finally {
        getItemSpy.mockRestore();
      }
    });
  });

  describe('getStoredCoreToken / storeCoreToken / clearStoredCoreToken', () => {
    it('returns null when no token is stored', () => {
      expect(getStoredCoreToken()).toBeNull();
    });

    it('returns the stored token', () => {
      localStorage.setItem(TOKEN_STORAGE_KEY, 'abc-123');
      expect(getStoredCoreToken()).toBe('abc-123');
    });

    it('trims whitespace around the stored token', () => {
      localStorage.setItem(TOKEN_STORAGE_KEY, '   xyz   ');
      expect(getStoredCoreToken()).toBe('xyz');
    });

    it('treats whitespace-only / empty stored values as null', () => {
      localStorage.setItem(TOKEN_STORAGE_KEY, '   ');
      expect(getStoredCoreToken()).toBeNull();
      localStorage.setItem(TOKEN_STORAGE_KEY, '');
      expect(getStoredCoreToken()).toBeNull();
    });

    it('storeCoreToken persists trimmed value', () => {
      storeCoreToken('  hello  ');
      expect(localStorage.getItem(TOKEN_STORAGE_KEY)).toBe('hello');
    });

    it('storeCoreToken with empty string clears the stored value', () => {
      localStorage.setItem(TOKEN_STORAGE_KEY, 'something');
      storeCoreToken('');
      expect(localStorage.getItem(TOKEN_STORAGE_KEY)).toBeNull();
    });

    it('clearStoredCoreToken removes the value', () => {
      localStorage.setItem(TOKEN_STORAGE_KEY, 'something');
      clearStoredCoreToken();
      expect(localStorage.getItem(TOKEN_STORAGE_KEY)).toBeNull();
    });

    it('returns null when localStorage is unavailable', () => {
      const getItemSpy = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
        throw new Error('blocked');
      });
      try {
        expect(getStoredCoreToken()).toBeNull();
      } finally {
        getItemSpy.mockRestore();
      }
    });
  });

  describe('peekStoredRpcUrl', () => {
    it('returns null when nothing is stored', () => {
      expect(peekStoredRpcUrl()).toBeNull();
    });

    it('returns the stored value (trimmed) — even when it equals the build-time default', () => {
      // Regression: legacy `getStoredRpcUrl !== CORE_RPC_URL` check threw away
      // user-explicit URLs that happened to equal the default, silently
      // routing cloud-mode RPC back to the local sidecar.
      localStorage.setItem(STORAGE_KEY, '  http://127.0.0.1:7788/rpc  ');
      expect(peekStoredRpcUrl()).toBe('http://127.0.0.1:7788/rpc');
    });
  });

  describe('getStoredCoreMode / storeCoreMode / clearStoredCoreMode', () => {
    it('returns null by default', () => {
      expect(getStoredCoreMode()).toBeNull();
    });

    it('round-trips local and cloud markers', () => {
      storeCoreMode('local');
      expect(getStoredCoreMode()).toBe('local');
      storeCoreMode('cloud');
      expect(getStoredCoreMode()).toBe('cloud');
    });

    it('treats unrecognised stored values as null', () => {
      localStorage.setItem(MODE_STORAGE_KEY, 'gibberish');
      expect(getStoredCoreMode()).toBeNull();
    });

    it('clearStoredCoreMode removes the marker', () => {
      storeCoreMode('cloud');
      clearStoredCoreMode();
      expect(getStoredCoreMode()).toBeNull();
    });
  });
});
