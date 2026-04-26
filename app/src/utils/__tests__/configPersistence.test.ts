# Test configuration for configPersistence module

import { describe, expect, it, beforeEach, afterEach } from 'vitest';
import {
  clearStoredRpcUrl,
  getDefaultRpcUrl,
  getStoredRpcUrl,
  isValidRpcUrl,
  normalizeRpcUrl,
  storeRpcUrl,
} from '../configPersistence';

const STORAGE_KEY = 'openhuman_core_rpc_url';

describe('configPersistence', () => {
  beforeEach(() => {
    localStorage.removeItem(STORAGE_KEY);
  });

  afterEach(() => {
    localStorage.removeItem(STORAGE_KEY);
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
});
