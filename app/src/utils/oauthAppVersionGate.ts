import { getVersion } from '@tauri-apps/api/app';
import { isTauri } from '@tauri-apps/api/core';

import { LATEST_APP_DOWNLOAD_URL, MINIMUM_SUPPORTED_APP_VERSION } from './config';
import { isVersionAtLeast, parseSemverParts } from './semver';

export type OAuthAppVersionGateResult =
  | { ok: true }
  | { ok: false; current: string; minimum: string; downloadUrl: string };

/**
 * When `VITE_MINIMUM_SUPPORTED_APP_VERSION` is set (CI/production), block OAuth
 * `openhuman://oauth/success` handling if the running desktop build is older.
 * Prevents completing Gmail (and other) OAuth on deprecated app binaries.
 */
export async function evaluateOAuthAppVersionGate(): Promise<OAuthAppVersionGateResult> {
  try {
    const minimum = MINIMUM_SUPPORTED_APP_VERSION.trim();
    if (!minimum) {
      return { ok: true };
    }
    if (!parseSemverParts(minimum)) {
      console.warn('[oauth-app-version] invalid MINIMUM_SUPPORTED_APP_VERSION; gate disabled');
      return { ok: true };
    }
    if (!isTauri()) {
      return { ok: true };
    }

    let current: string;
    try {
      current = await getVersion();
    } catch (e) {
      console.warn('[oauth-app-version] getVersion failed; allowing OAuth', e);
      return { ok: true };
    }

    if (!parseSemverParts(current)) {
      console.warn('[oauth-app-version] unparseable app version; allowing OAuth', current);
      return { ok: true };
    }

    if (isVersionAtLeast(current, minimum)) {
      return { ok: true };
    }

    console.warn('[oauth-app-version] blocked OAuth success deep link', { current, minimum });
    return { ok: false, current, minimum, downloadUrl: LATEST_APP_DOWNLOAD_URL };
  } catch (e) {
    // Never throw: outer deep-link handler logs the raw URL on failure, which can include secrets (e.g. clientKey).
    console.warn('[oauth-app-version] unexpected error; allowing OAuth', e);
    return { ok: true };
  }
}
