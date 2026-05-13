import { getVersion } from '@tauri-apps/api/app';

import { LATEST_APP_DOWNLOAD_URL, MINIMUM_SUPPORTED_APP_VERSION } from './config';
import { isVersionAtLeast, parseSemverParts } from './semver';
import { isTauri } from './tauriCommands/common';

export type OAuthAppVersionGateResult =
  | { ok: true }
  | { ok: false; current: string; minimum: string; downloadUrl: string };

function block(minimum: string, current: string): OAuthAppVersionGateResult {
  return { ok: false, current, minimum, downloadUrl: LATEST_APP_DOWNLOAD_URL };
}

/**
 * When `VITE_MINIMUM_SUPPORTED_APP_VERSION` is set (CI/production), block OAuth
 * `openhuman://oauth/success` handling if the running desktop build is older.
 * Prevents completing Gmail (and other) OAuth on deprecated app binaries.
 *
 * When a minimum is configured, fails **closed** if the app version cannot be
 * determined or parsed (never silently allows OAuth on unknown versions).
 */
export async function evaluateOAuthAppVersionGate(): Promise<OAuthAppVersionGateResult> {
  const minimum = MINIMUM_SUPPORTED_APP_VERSION.trim();
  try {
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
      console.warn('[oauth-app-version] getVersion failed; blocking OAuth', e);
      return block(minimum, 'unknown');
    }

    if (!parseSemverParts(current)) {
      console.warn('[oauth-app-version] unparseable app version; blocking OAuth', current);
      return block(minimum, current);
    }

    if (isVersionAtLeast(current, minimum)) {
      return { ok: true };
    }

    console.warn('[oauth-app-version] blocked OAuth success deep link', { current, minimum });
    return block(minimum, current);
  } catch (e) {
    // Never throw: outer deep-link handler must not receive errors that could log the raw URL.
    console.warn('[oauth-app-version] unexpected error', e);
    if (!minimum) {
      return { ok: true };
    }
    return block(minimum, 'unknown');
  }
}
