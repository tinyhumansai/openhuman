/**
 * Boot-check orchestrator.
 *
 * Runs before the main app mounts to verify that the active core mode is
 * reachable and version-compatible.  The caller (BootCheckGate) supplies the
 * current CoreMode from Redux and renders the appropriate recovery UI based on
 * the returned BootCheckResult.
 *
 * Design constraints:
 *  - Pure logic — no React, no Redux imports.
 *  - Injectable transport (callRpc / invokeCmd) for hermetic unit tests.
 *  - All branches emit [boot-check] prefixed debug logs.
 */
import debug from 'debug';

import { clearCoreRpcUrlCache } from '../../services/coreRpcClient';
import type { CoreMode } from '../../store/coreModeSlice';
import { APP_VERSION } from '../../utils/config';
import { storeRpcUrl } from '../../utils/configPersistence';

const log = debug('boot-check');
const logError = debug('boot-check:error');

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

export type BootCheckResult =
  | { kind: 'match' }
  | { kind: 'daemonDetected' }
  | { kind: 'outdatedLocal' }
  | { kind: 'outdatedCloud' }
  | { kind: 'noVersionMethod' }
  | { kind: 'unreachable'; reason: string };

// ---------------------------------------------------------------------------
// Transport interface (injectable for tests)
// ---------------------------------------------------------------------------

export interface BootCheckTransport {
  /** Call a JSON-RPC method on the active core endpoint. */
  callRpc: <T>(method: string, params?: Record<string, unknown>) => Promise<T>;
  /** Invoke a Tauri command. */
  invokeCmd: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
}

// ---------------------------------------------------------------------------
// Default transport (real app)
// ---------------------------------------------------------------------------

async function defaultCallRpc<T>(method: string, params?: Record<string, unknown>): Promise<T> {
  // Imported lazily via the default-real path so the module can be used in
  // non-Tauri test environments without side-effects.
  const { callCoreRpc } = await import('../../services/coreRpcClient');
  return callCoreRpc<T>({ method, params });
}

async function defaultInvokeCmd<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<T>(cmd, args);
}

export const defaultTransport: BootCheckTransport = {
  callRpc: defaultCallRpc,
  invokeCmd: defaultInvokeCmd,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Returns true if err looks like a JSON-RPC -32601 "Method not found". */
function isMethodNotFound(err: unknown): boolean {
  if (!err) return false;
  const msg = err instanceof Error ? err.message : String(err);
  return (
    msg.includes('-32601') ||
    msg.toLowerCase().includes('method not found') ||
    msg.toLowerCase().includes('methodnotfound')
  );
}

/**
 * Poll `openhuman.ping` with exponential back-off until the core responds or
 * we exhaust the budget.
 *
 * Returns true when the core is reachable, false on timeout.
 */
async function waitForCore(
  callRpc: BootCheckTransport['callRpc'],
  maxMs = 10_000
): Promise<boolean> {
  const delays = [200, 400, 800, 1000, 1000, 1000, 1000, 1000, 1000, 1000];
  let elapsed = 0;
  for (const delay of delays) {
    try {
      log('[boot-check] ping attempt elapsed=%dms', elapsed);
      await callRpc('openhuman.ping', {});
      log('[boot-check] ping succeeded elapsed=%dms', elapsed);
      return true;
    } catch {
      elapsed += delay;
      if (elapsed >= maxMs) break;
      await new Promise(r => setTimeout(r, delay));
    }
  }
  logError('[boot-check] ping timed out after %dms', elapsed);
  return false;
}

/**
 * Check `openhuman.service_status`.  Returns true when a separate
 * background daemon (distinct from our embedded core) is detected.
 */
async function isDaemonRunning(callRpc: BootCheckTransport['callRpc']): Promise<boolean> {
  try {
    const result = await callRpc<{ installed?: boolean; running?: boolean }>(
      'openhuman.service_status',
      {}
    );
    const detected = Boolean(result?.installed || result?.running);
    log(
      '[boot-check] service_status detected=%s installed=%s running=%s',
      detected,
      result?.installed,
      result?.running
    );
    return detected;
  } catch (err) {
    log('[boot-check] service_status error (non-fatal): %o', err);
    return false;
  }
}

/**
 * Fetch the running core version and compare it to the app build version.
 *
 * Returns:
 *   'match'           — versions are equal
 *   'outdated'        — version mismatch
 *   'noVersionMethod' — core responded but doesn't know the method
 *   'unreachable'     — network-level failure
 */
type VersionCheckResult = 'match' | 'outdated' | 'noVersionMethod' | 'unreachable';

async function checkVersion(callRpc: BootCheckTransport['callRpc']): Promise<VersionCheckResult> {
  try {
    const result = await callRpc<{ version_info?: { version?: string } }>(
      'openhuman.update_version',
      {}
    );
    const coreVersion = result?.version_info?.version ?? '';
    log('[boot-check] version_check app=%s core=%s', APP_VERSION, coreVersion);

    if (!coreVersion) {
      // Response received but no version field — treat like outdated.
      logError('[boot-check] update_version returned no version field');
      return 'outdated';
    }

    return coreVersion === APP_VERSION ? 'match' : 'outdated';
  } catch (err) {
    if (isMethodNotFound(err)) {
      log('[boot-check] update_version method not found (-32601)');
      return 'noVersionMethod';
    }
    logError('[boot-check] update_version call failed: %o', err);
    return 'unreachable';
  }
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/**
 * Run the boot-check for a given core mode.
 *
 * Local mode:
 *   1. Invoke `start_core_process` Tauri command to spawn the embedded core.
 *   2. Poll `openhuman.ping` until reachable (≤10 s).
 *   3. Check for a legacy daemon via `service_status`.
 *   4. Version-check via `update_version`.
 *
 * Cloud mode:
 *   1. Store the URL override and bust the RPC URL cache.
 *   2. Version-check via `update_version`.
 */
export async function runBootCheck(
  mode: CoreMode,
  transport: BootCheckTransport = defaultTransport
): Promise<BootCheckResult> {
  const { callRpc, invokeCmd } = transport;

  if (mode.kind === 'unset') {
    // Should never be called with unset — gate should show picker instead.
    logError('[boot-check] runBootCheck called with mode=unset (bug in caller)');
    return { kind: 'unreachable', reason: 'No core mode selected' };
  }

  // ------------------------------------------------------------------
  // Local mode
  // ------------------------------------------------------------------
  if (mode.kind === 'local') {
    log('[boot-check] local mode — starting core process');

    try {
      await invokeCmd<void>('start_core_process', {});
      log('[boot-check] start_core_process invoked successfully');
    } catch (err) {
      logError('[boot-check] start_core_process failed: %o', err);
      return {
        kind: 'unreachable',
        reason: `Failed to start local core: ${err instanceof Error ? err.message : String(err)}`,
      };
    }

    // Wait for the embedded core to be reachable.
    const reachable = await waitForCore(callRpc);
    if (!reachable) {
      logError('[boot-check] local core unreachable after retries');
      return { kind: 'unreachable', reason: 'Local core did not respond in time' };
    }

    // Check for a legacy background daemon that should be removed.
    const daemonDetected = await isDaemonRunning(callRpc);
    if (daemonDetected) {
      log('[boot-check] legacy daemon detected');
      return { kind: 'daemonDetected' };
    }

    // Version check.
    const versionResult = await checkVersion(callRpc);
    if (versionResult === 'match') {
      log('[boot-check] local mode — version match, boot complete');
      return { kind: 'match' };
    }
    if (versionResult === 'noVersionMethod') {
      log('[boot-check] local mode — noVersionMethod');
      return { kind: 'noVersionMethod' };
    }
    if (versionResult === 'unreachable') {
      logError('[boot-check] local mode — version check unreachable');
      return { kind: 'unreachable', reason: 'Could not reach core version endpoint' };
    }
    log('[boot-check] local mode — version outdated');
    return { kind: 'outdatedLocal' };
  }

  // ------------------------------------------------------------------
  // Cloud mode
  // ------------------------------------------------------------------
  log('[boot-check] cloud mode — url=%s', mode.url);
  storeRpcUrl(mode.url);
  clearCoreRpcUrlCache();
  log('[boot-check] cloud RPC URL stored and cache cleared');

  const versionResult = await checkVersion(callRpc);
  if (versionResult === 'match') {
    log('[boot-check] cloud mode — version match, boot complete');
    return { kind: 'match' };
  }
  if (versionResult === 'noVersionMethod') {
    log('[boot-check] cloud mode — noVersionMethod');
    return { kind: 'noVersionMethod' };
  }
  if (versionResult === 'unreachable') {
    logError('[boot-check] cloud mode — core unreachable');
    return { kind: 'unreachable', reason: 'Could not reach cloud core' };
  }
  log('[boot-check] cloud mode — version outdated');
  return { kind: 'outdatedCloud' };
}
