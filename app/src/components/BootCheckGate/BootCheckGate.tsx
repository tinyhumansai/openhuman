/**
 * BootCheckGate — pre-router gate rendered before the rest of the app mounts.
 *
 * Responsibilities:
 *   1. First-ever launch: prompt user to pick Local or Cloud core mode.
 *   2. Subsequent launches: run version / reachability check and block until
 *      the result is `match`.
 *
 * Visual language follows ServiceBlockingGate.tsx (bg-stone-950/80 overlay,
 * bg-stone-900 panel, ocean-500 / coral-500 semantics).
 */
import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { type BootCheckResult, runBootCheck } from '../../lib/bootCheck';
import { bootCheckTransport } from '../../services/bootCheckService';
import {
  clearCoreRpcTokenCache,
  clearCoreRpcUrlCache,
  testCoreRpcConnection,
} from '../../services/coreRpcClient';
import { type CoreMode, resetCoreMode, setCoreMode } from '../../store/coreModeSlice';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import {
  clearStoredCoreMode,
  clearStoredCoreToken,
  storeCoreMode,
  storeCoreToken,
  storeRpcUrl,
} from '../../utils/configPersistence';
import { isTauri } from '../../utils/tauriCommands/common';

const log = debug('boot-check');
const logError = debug('boot-check:error');

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

type Phase =
  | 'picker' // mode not set — show mode selector
  | 'checking' // boot check in flight
  | 'result'; // check finished with a non-match result

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

interface PanelProps {
  children: React.ReactNode;
}

function Panel({ children }: PanelProps) {
  return (
    <div className="fixed inset-0 z-[10000] bg-stone-950/80 backdrop-blur-sm flex items-center justify-center p-4">
      <div className="w-full max-w-xl rounded-2xl border border-stone-700/50 bg-stone-900 p-6 shadow-2xl">
        {children}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Picker (first-ever launch)
// ---------------------------------------------------------------------------

interface PickerProps {
  onConfirm: (mode: CoreMode) => void;
}

type TestStatus =
  | { kind: 'idle' }
  | { kind: 'testing' }
  | { kind: 'ok' }
  | { kind: 'auth' }
  | { kind: 'unreachable'; reason: string };

// Desktop release artifact URL surfaced on the web build's mode picker so
// users without a remote core have a clear path to install the app instead
// of being trapped on the cloud-only form.
const DESKTOP_DOWNLOAD_URL = 'https://github.com/tinyhumansai/openhuman/releases/latest';

function ModePicker({ onConfirm }: PickerProps) {
  // Web build cannot spawn a local sidecar, so the only viable choice is
  // cloud. Default the selection accordingly and hide the local option in
  // the render path below.
  const isDesktop = isTauri();
  const [selected, setSelected] = useState<'local' | 'cloud'>(isDesktop ? 'local' : 'cloud');
  const [cloudUrl, setCloudUrl] = useState('');
  const [cloudToken, setCloudToken] = useState('');
  const [urlError, setUrlError] = useState<string | null>(null);
  const [tokenError, setTokenError] = useState<string | null>(null);
  const [testStatus, setTestStatus] = useState<TestStatus>({ kind: 'idle' });

  /**
   * Validate the cloud URL + token inputs against a live core before we
   * commit the mode. We hit the public `core.ping` (auth-bypass) to confirm
   * reachability, then re-issue the same JSON-RPC envelope with the bearer
   * token to confirm `/rpc` accepts it. This catches the two most common
   * paste-time mistakes — wrong URL, wrong/missing token — with one click,
   * before the user lands on the unreachable result screen.
   *
   * Tokens are never logged: only `tokenLen` is emitted via the existing
   * picker debug line, and any error messages from the network/JSON parse
   * paths are passed through verbatim without the bearer value.
   */
  const validateInputs = (): { url: string; token: string } | null => {
    const trimmedUrl = cloudUrl.trim();
    if (!trimmedUrl) {
      setUrlError('Please enter a core URL.');
      return null;
    }
    try {
      const parsed = new URL(trimmedUrl);
      if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
        setUrlError('URL must start with http:// or https://');
        return null;
      }
    } catch {
      setUrlError('Please enter a valid URL (e.g. https://core.example.com/rpc)');
      return null;
    }
    setUrlError(null);

    const trimmedToken = cloudToken.trim();
    if (!trimmedToken) {
      setTokenError('Please enter the core auth token.');
      return null;
    }
    setTokenError(null);

    return { url: trimmedUrl, token: trimmedToken };
  };

  const handleTestConnection = async () => {
    const validated = validateInputs();
    if (!validated) return;

    setTestStatus({ kind: 'testing' });
    log(
      '[boot-check] picker — testing cloud connection url=%s tokenLen=%d',
      validated.url,
      validated.token.length
    );

    try {
      const response = await testCoreRpcConnection(validated.url, validated.token);
      if (response.status === 401 || response.status === 403) {
        log('[boot-check] picker — test failed: auth (status=%d)', response.status);
        setTestStatus({ kind: 'auth' });
        return;
      }
      if (!response.ok) {
        log('[boot-check] picker — test failed: HTTP %d', response.status);
        setTestStatus({ kind: 'unreachable', reason: `HTTP ${response.status} from /rpc` });
        return;
      }
      // Drain the body — response.ok with JSON-RPC error is still reachable.
      try {
        await response.json();
      } catch {
        // Non-JSON body is unusual but doesn't disprove reachability.
      }
      log('[boot-check] picker — test succeeded');
      setTestStatus({ kind: 'ok' });
    } catch (err) {
      const reason = err instanceof Error ? err.message : 'Connection failed';
      logError('[boot-check] picker — test errored: %o', err);
      setTestStatus({ kind: 'unreachable', reason });
    }
  };

  const handleContinue = () => {
    if (selected === 'local') {
      log('[boot-check] picker — user selected local mode');
      onConfirm({ kind: 'local' });
      return;
    }

    const validated = validateInputs();
    if (!validated) return;

    log(
      '[boot-check] picker — user selected cloud mode url=%s tokenLen=%d',
      validated.url,
      validated.token.length
    );
    onConfirm({ kind: 'cloud', url: validated.url, token: validated.token });
  };

  return (
    <Panel>
      <h2 className="text-xl font-semibold text-white">
        {isDesktop ? 'Choose core mode' : 'Connect to your core'}
      </h2>
      <p className="mt-2 text-sm text-stone-300">
        {isDesktop
          ? 'OpenHuman needs a running core to operate. Choose how you want to connect.'
          : 'OpenHuman on the web connects to a remote core you control. Enter its URL and auth token, or install the desktop app to run one locally.'}
      </p>

      {!isDesktop && (
        <div
          className="mt-4 rounded-xl border border-stone-700 bg-stone-800/60 p-3 text-xs text-stone-300"
          data-testid="web-download-cta">
          Prefer to run everything on your own device?{' '}
          <a
            href={DESKTOP_DOWNLOAD_URL}
            target="_blank"
            rel="noopener noreferrer"
            className="text-ocean-400 underline hover:text-ocean-300">
            Download the desktop app
          </a>
          .
        </div>
      )}

      <div className="mt-5 flex flex-col gap-3">
        {/* Local option — desktop only; web builds cannot spawn a sidecar. */}
        {isDesktop && (
          <button
            type="button"
            onClick={() => setSelected('local')}
            className={`rounded-xl border p-4 text-left transition-colors ${
              selected === 'local'
                ? 'border-ocean-500 bg-ocean-500/10 text-white'
                : 'border-stone-700 text-stone-300 hover:border-stone-500 hover:bg-stone-800'
            }`}>
            <div className="font-medium">Local (recommended)</div>
            <div className="mt-0.5 text-xs text-stone-400">
              Embedded core runs on this device — fastest, no configuration required.
            </div>
          </button>
        )}

        {/* Cloud option — always available; the only option on the web build. */}
        {isDesktop && (
          <button
            type="button"
            onClick={() => setSelected('cloud')}
            className={`rounded-xl border p-4 text-left transition-colors ${
              selected === 'cloud'
                ? 'border-ocean-500 bg-ocean-500/10 text-white'
                : 'border-stone-700 text-stone-300 hover:border-stone-500 hover:bg-stone-800'
            }`}>
            <div className="font-medium">Cloud</div>
            <div className="mt-0.5 text-xs text-stone-400">
              Connect to a remote core at a custom URL.
            </div>
          </button>
        )}

        {selected === 'cloud' && (
          <div className="mt-1 flex flex-col gap-3">
            <div className="flex flex-col gap-1">
              <label className="text-xs font-medium text-stone-300">Core RPC URL</label>
              <input
                type="url"
                placeholder="https://core.example.com/rpc"
                value={cloudUrl}
                onChange={e => {
                  setCloudUrl(e.target.value);
                  setUrlError(null);
                  setTestStatus({ kind: 'idle' });
                }}
                className="rounded-lg border border-stone-600 bg-stone-800 px-3 py-2 text-sm text-white placeholder-stone-500 focus:border-ocean-500 focus:outline-none"
              />
              {urlError && <p className="text-xs text-coral-400">{urlError}</p>}
            </div>
            <div className="flex flex-col gap-1">
              <label className="text-xs font-medium text-stone-300">
                Auth token (<code className="text-[10px]">OPENHUMAN_CORE_TOKEN</code>)
              </label>
              <input
                type="password"
                autoComplete="off"
                spellCheck={false}
                placeholder="Bearer token configured on the remote core"
                value={cloudToken}
                onChange={e => {
                  setCloudToken(e.target.value);
                  setTokenError(null);
                  setTestStatus({ kind: 'idle' });
                }}
                className="rounded-lg border border-stone-600 bg-stone-800 px-3 py-2 text-sm text-white placeholder-stone-500 focus:border-ocean-500 focus:outline-none"
              />
              {tokenError && <p className="text-xs text-coral-400">{tokenError}</p>}
              <p className="text-[11px] text-stone-500">
                Stored on this device only. Required for remote cores — the desktop sends it as{' '}
                <code>Authorization: Bearer …</code> on every RPC.
              </p>
            </div>

            <div className="flex items-center gap-3">
              <button
                type="button"
                onClick={handleTestConnection}
                disabled={testStatus.kind === 'testing'}
                className="rounded-lg border border-stone-600 px-3 py-1.5 text-xs text-stone-100 hover:bg-stone-800 disabled:opacity-60">
                {testStatus.kind === 'testing' ? 'Testing…' : 'Test connection'}
              </button>
              {testStatus.kind === 'ok' && (
                <span className="text-xs text-emerald-400" data-testid="test-status-ok">
                  Connected ✓
                </span>
              )}
              {testStatus.kind === 'auth' && (
                <span className="text-xs text-coral-400" data-testid="test-status-auth">
                  Auth failed — check the token (got 401/403).
                </span>
              )}
              {testStatus.kind === 'unreachable' && (
                <span className="text-xs text-coral-400" data-testid="test-status-unreachable">
                  Unreachable: {testStatus.reason}
                </span>
              )}
            </div>
          </div>
        )}
      </div>

      <div className="mt-6 flex justify-end">
        <button
          type="button"
          onClick={handleContinue}
          className="rounded-lg bg-ocean-500 px-5 py-2 text-sm font-medium text-white hover:bg-ocean-600">
          Continue
        </button>
      </div>
    </Panel>
  );
}

// ---------------------------------------------------------------------------
// Spinner / checking
// ---------------------------------------------------------------------------

function CheckingScreen() {
  return (
    <Panel>
      <div className="flex flex-col items-center gap-4 py-4">
        <div className="h-8 w-8 animate-spin rounded-full border-2 border-stone-600 border-t-ocean-500" />
        <p className="text-sm text-stone-300">Checking core…</p>
      </div>
    </Panel>
  );
}

// ---------------------------------------------------------------------------
// Result screens
// ---------------------------------------------------------------------------

interface ResultScreenProps {
  result: BootCheckResult;
  onRetry: () => void;
  onSwitchMode: () => void;
  onQuit: () => void;
  actionBusy: boolean;
  actionError: string | null;
  onAction: () => void;
}

function ResultScreen({
  result,
  onRetry,
  onSwitchMode,
  onQuit,
  actionBusy,
  actionError,
  onAction,
}: ResultScreenProps) {
  if (result.kind === 'match') return null;

  if (result.kind === 'unreachable') {
    return (
      <Panel>
        <h2 className="text-xl font-semibold text-white">Could not reach core</h2>
        <p className="mt-2 text-sm text-stone-300">
          {result.reason || 'The core process is unreachable. Try switching to a different mode.'}
        </p>
        {actionError && <p className="mt-3 text-xs text-coral-400">{actionError}</p>}
        <div className="mt-5 flex gap-3">
          <button
            type="button"
            onClick={onRetry}
            disabled={actionBusy}
            className="rounded-lg border border-stone-600 px-4 py-2 text-sm text-stone-100 hover:bg-stone-800 disabled:opacity-60">
            Retry
          </button>
          <button
            type="button"
            onClick={onSwitchMode}
            className="rounded-lg border border-stone-600 px-4 py-2 text-sm text-stone-100 hover:bg-stone-800">
            Switch mode
          </button>
          <button
            type="button"
            onClick={onQuit}
            className="rounded-lg bg-coral-500 px-4 py-2 text-sm font-medium text-white hover:bg-coral-600">
            Quit
          </button>
        </div>
      </Panel>
    );
  }

  if (result.kind === 'daemonDetected') {
    return (
      <Panel>
        <h2 className="text-xl font-semibold text-white">Legacy background core detected</h2>
        <p className="mt-2 text-sm text-stone-300">
          A separately-installed OpenHuman daemon is running on this device. It must be removed
          before the embedded core can take over.
        </p>
        {actionError && <p className="mt-3 text-xs text-coral-400">{actionError}</p>}
        <div className="mt-5 flex gap-3">
          <button
            type="button"
            onClick={onAction}
            disabled={actionBusy}
            className="rounded-lg bg-coral-500 px-4 py-2 text-sm font-medium text-white hover:bg-coral-600 disabled:opacity-60">
            {actionBusy ? 'Removing…' : 'Remove and continue'}
          </button>
          <button
            type="button"
            onClick={onSwitchMode}
            disabled={actionBusy}
            className="rounded-lg border border-stone-600 px-4 py-2 text-sm text-stone-100 hover:bg-stone-800 disabled:opacity-60">
            Switch mode
          </button>
        </div>
      </Panel>
    );
  }

  if (result.kind === 'outdatedLocal') {
    return (
      <Panel>
        <h2 className="text-xl font-semibold text-white">Local core needs a restart</h2>
        <p className="mt-2 text-sm text-stone-300">
          The local core version does not match this app build. Restarting it will load the correct
          version.
        </p>
        {actionError && <p className="mt-3 text-xs text-coral-400">{actionError}</p>}
        <div className="mt-5 flex gap-3">
          <button
            type="button"
            onClick={onAction}
            disabled={actionBusy}
            className="rounded-lg bg-ocean-500 px-4 py-2 text-sm font-medium text-white hover:bg-ocean-600 disabled:opacity-60">
            {actionBusy ? 'Restarting…' : 'Restart core'}
          </button>
          <button
            type="button"
            onClick={onSwitchMode}
            disabled={actionBusy}
            className="rounded-lg border border-stone-600 px-4 py-2 text-sm text-stone-100 hover:bg-stone-800 disabled:opacity-60">
            Switch mode
          </button>
        </div>
      </Panel>
    );
  }

  if (result.kind === 'outdatedCloud') {
    return (
      <Panel>
        <h2 className="text-xl font-semibold text-white">Cloud core needs an update</h2>
        <p className="mt-2 text-sm text-stone-300">
          The cloud core version does not match this app build. Run the core updater to resolve the
          mismatch.
        </p>
        {actionError && <p className="mt-3 text-xs text-coral-400">{actionError}</p>}
        <div className="mt-5 flex gap-3">
          <button
            type="button"
            onClick={onAction}
            disabled={actionBusy}
            className="rounded-lg bg-ocean-500 px-4 py-2 text-sm font-medium text-white hover:bg-ocean-600 disabled:opacity-60">
            {actionBusy ? 'Updating…' : 'Update cloud core'}
          </button>
          <button
            type="button"
            onClick={onSwitchMode}
            disabled={actionBusy}
            className="rounded-lg border border-stone-600 px-4 py-2 text-sm text-stone-100 hover:bg-stone-800 disabled:opacity-60">
            Switch mode
          </button>
        </div>
      </Panel>
    );
  }

  // noVersionMethod — treat like outdated, user picks which flavor of action
  return (
    <Panel>
      <h2 className="text-xl font-semibold text-white">Core version check failed</h2>
      <p className="mt-2 text-sm text-stone-300">
        The core is running but does not expose a version endpoint. It may be outdated. Restart or
        update the core to continue.
      </p>
      {actionError && <p className="mt-3 text-xs text-coral-400">{actionError}</p>}
      <div className="mt-5 flex gap-3">
        <button
          type="button"
          onClick={onAction}
          disabled={actionBusy}
          className="rounded-lg bg-ocean-500 px-4 py-2 text-sm font-medium text-white hover:bg-ocean-600 disabled:opacity-60">
          {actionBusy ? 'Working…' : 'Restart / update core'}
        </button>
        <button
          type="button"
          onClick={onSwitchMode}
          disabled={actionBusy}
          className="rounded-lg border border-stone-600 px-4 py-2 text-sm text-stone-100 hover:bg-stone-800 disabled:opacity-60">
          Switch mode
        </button>
      </div>
    </Panel>
  );
}

// ---------------------------------------------------------------------------
// Main gate
// ---------------------------------------------------------------------------

interface BootCheckGateProps {
  children: React.ReactNode;
}

export default function BootCheckGate({ children }: BootCheckGateProps) {
  const dispatch = useAppDispatch();
  const coreMode = useAppSelector(state => state.coreMode.mode);

  const [phase, setPhase] = useState<Phase>(() =>
    coreMode.kind === 'unset' ? 'picker' : 'checking'
  );
  const [result, setResult] = useState<BootCheckResult | null>(null);
  const [actionBusy, setActionBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  // Prevent concurrent or stale runs.
  const runningRef = useRef(false);

  // Production transport lives in services/bootCheckService so direct
  // Tauri/RPC imports stay localized there.
  const transport = bootCheckTransport;

  const runCheck = useCallback(
    async (mode: CoreMode) => {
      if (runningRef.current) {
        log('[boot-check] gate — check already running, skipping duplicate');
        return;
      }
      runningRef.current = true;
      setPhase('checking');
      setResult(null);
      setActionError(null);
      log('[boot-check] gate — starting check mode=%s', mode.kind);

      try {
        const checkResult = await runBootCheck(mode, transport);
        log('[boot-check] gate — check result=%s', checkResult.kind);

        if (checkResult.kind === 'match') {
          // Gate resolves — render children.
          setPhase('result');
          setResult(checkResult);
        } else {
          setPhase('result');
          setResult(checkResult);
        }
      } catch (err) {
        logError('[boot-check] gate — unexpected error: %o', err);
        setPhase('result');
        setResult({
          kind: 'unreachable',
          reason: err instanceof Error ? err.message : 'Unexpected boot-check error',
        });
      } finally {
        runningRef.current = false;
      }
    },
    // transport is stable (constructed inline but always same shape)
    // eslint-disable-next-line react-hooks/exhaustive-deps
    []
  );

  // Start check automatically when mode is set and we're in checking phase.
  // The async setState calls inside runCheck() happen after an await, so they
  // do not synchronously cascade — suppress the linter warning here.

  useEffect(() => {
    if (coreMode.kind !== 'unset' && phase === 'checking') {
      void runCheck(coreMode);
    }
  }, [coreMode, phase, runCheck]);

  // ------------------------------------------------------------------
  // Picker confirm — dispatches setCoreMode and kicks off check.
  // ------------------------------------------------------------------
  const handlePickerConfirm = useCallback(
    (mode: CoreMode) => {
      log('[boot-check] gate — picker confirmed mode=%s', mode.kind);
      // Persist URL + token for cloud mode so getCoreRpcUrl/Token resolve
      // correctly on the boot-check probe (and every subsequent RPC) without
      // waiting for redux-persist's async rehydrate to complete. Also write
      // the synchronous `openhuman_core_mode` marker so a reload triggered
      // mid-flight (e.g. `handleIdentityFlip` → `restartApp`) recovers the
      // chosen mode from localStorage before redux-persist flushes. Clear
      // caches so any prior local-mode resolution doesn't leak into cloud.
      if (mode.kind === 'cloud') {
        storeRpcUrl(mode.url);
        storeCoreToken(mode.token ?? '');
        storeCoreMode('cloud');
      } else {
        storeRpcUrl('');
        clearStoredCoreToken();
        storeCoreMode('local');
      }
      clearCoreRpcUrlCache();
      clearCoreRpcTokenCache();
      dispatch(setCoreMode(mode));
      setPhase('checking');
    },
    [dispatch]
  );

  // ------------------------------------------------------------------
  // Switch mode — reset to picker.
  // ------------------------------------------------------------------
  const handleSwitchMode = useCallback(() => {
    log('[boot-check] gate — switch mode requested');
    storeRpcUrl('');
    clearStoredCoreToken();
    clearStoredCoreMode();
    clearCoreRpcUrlCache();
    clearCoreRpcTokenCache();
    dispatch(resetCoreMode());
    setPhase('picker');
    setResult(null);
    setActionError(null);
  }, [dispatch]);

  // ------------------------------------------------------------------
  // Quit the app.
  // ------------------------------------------------------------------
  const handleQuit = useCallback(async () => {
    log('[boot-check] gate — quit requested');
    try {
      await bootCheckTransport.invokeCmd('app_quit');
    } catch (err) {
      logError('[boot-check] gate — app_quit failed: %o', err);
    }
  }, []);

  // ------------------------------------------------------------------
  // Retry (unreachable state).
  // ------------------------------------------------------------------
  const handleRetry = useCallback(() => {
    log('[boot-check] gate — retry requested');
    if (coreMode.kind !== 'unset') {
      runCheck(coreMode);
    }
  }, [coreMode, runCheck]);

  // ------------------------------------------------------------------
  // Primary action per result kind.
  // ------------------------------------------------------------------
  const handleAction = useCallback(async () => {
    if (!result || actionBusy) return;
    setActionBusy(true);
    setActionError(null);

    try {
      if (result.kind === 'daemonDetected') {
        log('[boot-check] gate — removing legacy daemon');
        await transport.callRpc('openhuman.service_stop', {});
        await transport.callRpc('openhuman.service_uninstall', {});
        log('[boot-check] gate — daemon removed, re-running check');
      } else if (result.kind === 'outdatedLocal' || result.kind === 'noVersionMethod') {
        log('[boot-check] gate — restarting local core');
        await transport.invokeCmd('restart_core_process', {});
        log('[boot-check] gate — local core restarted');
      } else if (result.kind === 'outdatedCloud') {
        log('[boot-check] gate — triggering cloud core update');
        await transport.callRpc('openhuman.update_run', {});
        log('[boot-check] gate — cloud core update triggered');
      }

      // Re-run the full check after the action.
      if (coreMode.kind !== 'unset') {
        runCheck(coreMode);
      }
    } catch (err) {
      logError('[boot-check] gate — action error: %o', err);
      setActionError(err instanceof Error ? err.message : 'Action failed — please try again.');
    } finally {
      setActionBusy(false);
    }
    // transport is stable shape
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [result, actionBusy, coreMode, runCheck]);

  // ------------------------------------------------------------------
  // Render
  // ------------------------------------------------------------------

  // Unset — show picker (even if Redux persisted something; phase reflects truth).
  if (phase === 'picker' || coreMode.kind === 'unset') {
    return (
      <>
        <ModePicker onConfirm={handlePickerConfirm} />
      </>
    );
  }

  // Check in flight.
  if (phase === 'checking') {
    return <CheckingScreen />;
  }

  // Match — pass through.
  if (result?.kind === 'match') {
    return <>{children}</>;
  }

  // Non-match result.
  return (
    <>
      <ResultScreen
        result={result ?? { kind: 'unreachable', reason: 'Unknown error' }}
        onRetry={handleRetry}
        onSwitchMode={handleSwitchMode}
        onQuit={handleQuit}
        actionBusy={actionBusy}
        actionError={actionError}
        onAction={handleAction}
      />
    </>
  );
}
