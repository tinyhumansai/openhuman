/**
 * Platform detection utilities.
 *
 * Uses navigator.userAgent plus isTauri() from webviewAccountService to decide
 * whether we are running inside the Tauri runtime on a phone (iOS or Android).
 *
 * For tests: override via setTestPlatform() / clearTestPlatform().
 * Production code must not call the override functions.
 */
import { isTauri } from '../services/webviewAccountService';

// -- test override -----------------------------------------------------------

type Platform = 'ios' | 'android' | 'desktop';

let _testOverride: Platform | null = null;

/**
 * Override the detected platform in tests.
 * Call clearTestPlatform() in afterEach to restore.
 */
export function setTestPlatform(platform: Platform): void {
  _testOverride = platform;
}

/** Restore automatic detection (call in afterEach). */
export function clearTestPlatform(): void {
  _testOverride = null;
}

// -- detection ---------------------------------------------------------------

function detectIOS(): boolean {
  if (_testOverride === 'ios') return true;
  if (_testOverride === 'android' || _testOverride === 'desktop') return false;

  if (typeof navigator === 'undefined') return false;

  const isMobileUA = /iPhone|iPad|iPod/i.test(navigator.userAgent);
  // Only treat as iOS when we're actually inside the Tauri runtime.
  // A web browser on an iPhone should not trigger iOS-specific Tauri flows.
  return isMobileUA && isTauri();
}

function detectAndroid(): boolean {
  if (_testOverride === 'android') return true;
  if (_testOverride === 'ios' || _testOverride === 'desktop') return false;

  if (typeof navigator === 'undefined') return false;

  const isAndroidUA = /Android/i.test(navigator.userAgent);
  return isAndroidUA && isTauri();
}

/**
 * True when the app is running on iOS (inside the Tauri iOS target).
 *
 * Evaluated lazily on first access and then cached for the lifetime of the
 * module — the platform never changes at runtime.
 */
let _isIOSCache: boolean | null = null;
let _isAndroidCache: boolean | null = null;

export function getIsIOS(): boolean {
  if (_testOverride !== null) {
    // Always re-evaluate when a test override is active.
    return detectIOS();
  }
  if (_isIOSCache === null) {
    _isIOSCache = detectIOS();
  }
  return _isIOSCache;
}

export function getIsAndroid(): boolean {
  if (_testOverride !== null) {
    return detectAndroid();
  }
  if (_isAndroidCache === null) {
    _isAndroidCache = detectAndroid();
  }
  return _isAndroidCache;
}

/** True for either mobile target (iOS or Android). */
export function getIsMobile(): boolean {
  return getIsIOS() || getIsAndroid();
}

/**
 * Convenience re-export as a constant.
 * Safe to import and use at module level — evaluated once on import.
 *
 * NOTE: if you need test overrides to work, call getIsIOS() instead,
 * since this is evaluated at module load time.
 */
export const isIOS: boolean = detectIOS();
export const isAndroid: boolean = detectAndroid();
