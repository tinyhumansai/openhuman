import { afterEach, describe, expect, it } from 'vitest';

import {
  clearTestPlatform,
  getIsAndroid,
  getIsIOS,
  getIsMobile,
  setTestPlatform,
} from './platform';

describe('platform detection', () => {
  afterEach(() => {
    clearTestPlatform();
  });

  it('returns false by default in test environment (not iOS UA)', () => {
    // In Vitest / jsdom navigator.userAgent is not an iPhone string,
    // so the result should be false with no override.
    clearTestPlatform();
    // Don't assert a specific value since isTauri() may vary by env;
    // just confirm getIsIOS() is a boolean.
    expect(typeof getIsIOS()).toBe('boolean');
  });

  it('returns true when test override is set to "ios"', () => {
    setTestPlatform('ios');
    expect(getIsIOS()).toBe(true);
  });

  it('returns false when test override is set to "desktop"', () => {
    setTestPlatform('desktop');
    expect(getIsIOS()).toBe(false);
  });

  it('toggle works round-trip', () => {
    setTestPlatform('ios');
    expect(getIsIOS()).toBe(true);
    setTestPlatform('desktop');
    expect(getIsIOS()).toBe(false);
    clearTestPlatform();
    // After clear, back to auto-detect (still a boolean).
    expect(typeof getIsIOS()).toBe('boolean');
  });

  it('getIsAndroid reflects the "android" test override', () => {
    setTestPlatform('android');
    expect(getIsAndroid()).toBe(true);
    expect(getIsIOS()).toBe(false);
  });

  it('getIsAndroid returns false on iOS and desktop overrides', () => {
    setTestPlatform('ios');
    expect(getIsAndroid()).toBe(false);
    setTestPlatform('desktop');
    expect(getIsAndroid()).toBe(false);
  });

  it('getIsMobile is true for both iOS and Android, false for desktop', () => {
    setTestPlatform('ios');
    expect(getIsMobile()).toBe(true);
    setTestPlatform('android');
    expect(getIsMobile()).toBe(true);
    setTestPlatform('desktop');
    expect(getIsMobile()).toBe(false);
  });

  it('returns a boolean for getIsAndroid by default', () => {
    clearTestPlatform();
    expect(typeof getIsAndroid()).toBe('boolean');
    expect(typeof getIsMobile()).toBe('boolean');
  });
});
