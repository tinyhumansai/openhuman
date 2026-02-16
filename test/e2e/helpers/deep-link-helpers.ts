/**
 * Deep-link trigger utilities for E2E tests.
 *
 * Uses macOS `open` command to fire the custom `alphahuman://` URL scheme,
 * which the built .app bundle picks up via its registered CFBundleURLSchemes.
 */
import { exec } from 'child_process';

/**
 * Trigger a deep link URL via the macOS `open` command.
 * Resolves once the OS has dispatched the URL (does NOT wait for the app to
 * finish handling it).
 *
 * @param {string} url
 * @returns {Promise<void>}
 */
export function triggerDeepLink(url: string): Promise<void> {
  return new Promise<void>((resolve, reject) => {
    exec(`open "${url}"`, error => {
      if (error) reject(new Error(`Failed to trigger deep link: ${error.message}`));
      else resolve();
    });
  });
}

/**
 * Convenience wrapper for auth deep links.
 *
 * @param {string} token - The login token to embed in the URL.
 * @returns {Promise<void>}
 */
export function triggerAuthDeepLink(token: string): Promise<void> {
  return triggerDeepLink(`alphahuman://auth?token=${encodeURIComponent(token)}`);
}
