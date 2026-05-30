import debug from 'debug';

import { getCoreStateSnapshot } from '../../lib/coreState/store';
import { bootCheckTransport } from '../../services/bootCheckService';
import { getCoreRpcUrl, testCoreRpcConnection } from '../../services/coreRpcClient';
import { isTauri } from '../../services/webviewAccountService';
import {
  getStoredCoreMode,
  getStoredCoreToken,
  storeCoreMode,
} from '../../utils/configPersistence';

const logPrefix = '[oauth-auth-readiness]';
const log = debug('oauth:auth-readiness');
const warnLog = debug('oauth:auth-readiness:warn');

const DEFAULT_MAX_WAIT_MS = 30_000;
const POLL_MS = 200;

export type OAuthAuthReadinessFailure = 'core_mode_unset' | 'core_unreachable';

export type OAuthAuthReadinessResult =
  | { ready: true }
  | { ready: false; reason: OAuthAuthReadinessFailure };

const delay = (ms: number): Promise<void> =>
  new Promise(resolve => {
    setTimeout(resolve, ms);
  });

async function pingCoreRpc(): Promise<boolean> {
  try {
    const rpcUrl = await getCoreRpcUrl();
    // In cloud mode, pass the stored cloud token explicitly to avoid
    // getCoreRpcToken() resolving to a stale local-core token. See issue #2377.
    const cloudToken = getStoredCoreMode() === 'cloud' ? getStoredCoreToken() : null;
    log(`${logPrefix} core.ping probe`, {
      rpcUrl,
      mode: getStoredCoreMode(),
      hasCloudToken: Boolean(cloudToken),
    });
    const response = cloudToken
      ? await testCoreRpcConnection(rpcUrl, cloudToken)
      : await testCoreRpcConnection(rpcUrl);
    return response.ok;
  } catch (err) {
    log(`${logPrefix} core.ping probe failed`, err);
    return false;
  }
}

async function ensureLocalCoreProcessStarted(): Promise<void> {
  if (!isTauri()) {
    return;
  }
  if (getStoredCoreMode() !== 'local') {
    return;
  }
  try {
    await bootCheckTransport.invokeCmd('start_core_process', {});
    log(`${logPrefix} start_core_process invoked`);
  } catch (err) {
    log(`${logPrefix} start_core_process skipped or failed`, err);
  }
}

/**
 * Block OAuth sign-in until the BootCheckGate has committed a core mode,
 * and the embedded core answers `core.ping`.
 *
 * First-launch sign-in often failed with a generic "Sign-in failed" because
 * the deep-link handler only waited ~1.5s while `isBootstrapping` stayed true
 * behind the runtime picker, then called RPC against a core that was not up yet.
 * The login callback itself can proceed before CoreStateProvider completes its
 * first snapshot refresh; requiring that UI bootstrap here deadlocks some E2E
 * and first-login paths where the callback is what creates the session.
 */
export async function waitForOAuthAuthReadiness(
  maxWaitMs = DEFAULT_MAX_WAIT_MS
): Promise<OAuthAuthReadinessResult> {
  const deadline = Date.now() + maxWaitMs;
  let sawCoreMode = false;

  while (Date.now() < deadline) {
    const mode = getStoredCoreMode();
    if (mode) {
      sawCoreMode = true;
      break;
    }
    // In the Tauri desktop app the core is always embedded locally. If the
    // picker hasn't run yet (e.g. first launch before BootCheckGate finishes,
    // or core mode was just cleared), default to 'local' so OAuth can proceed
    // without forcing the user to navigate back through the runtime picker.
    if (isTauri()) {
      storeCoreMode('local');
      sawCoreMode = true;
      break;
    }
    await delay(POLL_MS);
  }

  if (!sawCoreMode) {
    warnLog(`${logPrefix} timed out waiting for core mode selection`);
    return { ready: false, reason: 'core_mode_unset' };
  }

  await ensureLocalCoreProcessStarted();

  while (Date.now() < deadline) {
    const coreState = getCoreStateSnapshot();
    if (await pingCoreRpc()) {
      log(`${logPrefix} ready`, {
        authBootstrapComplete: !coreState.isBootstrapping,
        hasSessionToken: Boolean(coreState.snapshot.sessionToken),
        coreMode: getStoredCoreMode(),
      });
      return { ready: true };
    }

    await delay(POLL_MS);
  }

  if (!(await pingCoreRpc())) {
    warnLog(`${logPrefix} core RPC unreachable after ${maxWaitMs}ms`);
    return { ready: false, reason: 'core_unreachable' };
  }

  return { ready: true };
}

export function oauthAuthReadinessUserMessage(reason: OAuthAuthReadinessFailure): string {
  switch (reason) {
    case 'core_mode_unset':
      return (
        'Finish choosing how OpenHuman runs (tap Continue on the setup screen), ' +
        'then try signing in again.'
      );
    case 'core_unreachable': {
      const mode = getStoredCoreMode();
      if (mode === 'cloud') {
        return (
          'OpenHuman could not reach its remote (cloud) runtime. ' +
          'Check your RPC URL and token in Settings, then try signing in again.'
        );
      }
      return (
        'OpenHuman could not reach its local runtime. Quit and reopen the app, ' +
        'then try signing in again.'
      );
    }
    default:
      return 'Sign-in is still starting up. Wait a few seconds and try again.';
  }
}

/**
 * Lightweight preflight before opening the system browser for OAuth.
 * Blocks browser launch when the local auth runtime is not ready yet.
 * `waitForOAuthAuthReadiness()` starts the local core when needed.
 */
export async function prepareOAuthLoginLaunch(): Promise<void> {
  const quick = await waitForOAuthAuthReadiness(8_000);
  if (!quick.ready) {
    warnLog(`${logPrefix} pre-launch readiness`, quick);
    throw new Error(oauthAuthReadinessUserMessage(quick.reason));
  }
}
