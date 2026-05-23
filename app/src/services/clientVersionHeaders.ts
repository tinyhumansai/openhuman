import { getVersion } from '@tauri-apps/api/app';

import { APP_VERSION } from '../utils/config';
import { isTauri } from '../utils/tauriCommands/common';
import { callCoreRpc } from './coreRpcClient';

const CLIENT_VERSION_MAX_LENGTH = 64;

let tauriVersionPromise: Promise<string | null> | null = null;
let coreVersionPromise: Promise<string | null> | null = null;

export function sanitizeClientVersion(raw: string | null | undefined): string | null {
  const sanitized = String(raw ?? '')
    .trim()
    .replace(/[^0-9A-Za-z._+-]+/g, '')
    .slice(0, CLIENT_VERSION_MAX_LENGTH);

  return sanitized.length > 0 ? sanitized : null;
}

async function getTauriClientVersion(): Promise<string | null> {
  if (!isTauri()) {
    return null;
  }

  if (!tauriVersionPromise) {
    tauriVersionPromise = getVersion()
      .then(version => sanitizeClientVersion(version))
      .catch(() => {
        tauriVersionPromise = null;
        return null;
      });
  }

  return tauriVersionPromise;
}

async function getCoreClientVersion(): Promise<string | null> {
  if (!isTauri()) {
    return null;
  }

  if (!coreVersionPromise) {
    coreVersionPromise = callCoreRpc<{ result?: { version?: string } }>({
      method: 'openhuman.update_version',
      params: {},
    })
      .then(response => sanitizeClientVersion(response?.result?.version))
      .catch(() => {
        // Clear the cached promise so a later call can retry once core is up.
        coreVersionPromise = null;
        return null;
      });
  }

  return coreVersionPromise;
}

export async function getClientVersionHeaders(): Promise<Record<string, string>> {
  if (isTauri()) {
    const [tauriVersion, coreVersion] = await Promise.all([
      getTauriClientVersion(),
      getCoreClientVersion(),
    ]);
    const headers: Record<string, string> = {};
    if (tauriVersion) headers['x-tauri-version'] = tauriVersion;
    if (coreVersion) headers['x-core-version'] = coreVersion;
    return headers;
  }

  const webVersion = sanitizeClientVersion(APP_VERSION);
  return webVersion ? { 'x-web-version': webVersion } : {};
}
