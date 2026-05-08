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
import { clearCoreRpcUrlCache } from '../../services/coreRpcClient';
import { type CoreMode, resetCoreMode, setCoreMode } from '../../store/coreModeSlice';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import { storeRpcUrl } from '../../utils/configPersistence';

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

function ModePicker({ onConfirm }: PickerProps) {
  const [selected, setSelected] = useState<'local' | 'cloud'>('local');
  const [cloudUrl, setCloudUrl] = useState('');
  const [urlError, setUrlError] = useState<string | null>(null);

  const handleContinue = () => {
    if (selected === 'local') {
      log('[boot-check] picker — user selected local mode');
      onConfirm({ kind: 'local' });
      return;
    }

    // Basic URL validation: must be http(s)
    const trimmed = cloudUrl.trim();
    if (!trimmed) {
      setUrlError('Please enter a core URL.');
      return;
    }
    try {
      const parsed = new URL(trimmed);
      if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
        setUrlError('URL must start with http:// or https://');
        return;
      }
    } catch {
      setUrlError('Please enter a valid URL (e.g. https://core.example.com/rpc)');
      return;
    }

    setUrlError(null);
    log('[boot-check] picker — user selected cloud mode url=%s', trimmed);
    onConfirm({ kind: 'cloud', url: trimmed });
  };

  return (
    <Panel>
      <h2 className="text-xl font-semibold text-white">Choose core mode</h2>
      <p className="mt-2 text-sm text-stone-300">
        OpenHuman needs a running core to operate. Choose how you want to connect.
      </p>

      <div className="mt-5 flex flex-col gap-3">
        {/* Local option */}
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

        {/* Cloud option */}
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

        {selected === 'cloud' && (
          <div className="mt-1 flex flex-col gap-1">
            <input
              type="url"
              placeholder="https://core.example.com/rpc"
              value={cloudUrl}
              onChange={e => {
                setCloudUrl(e.target.value);
                setUrlError(null);
              }}
              className="rounded-lg border border-stone-600 bg-stone-800 px-3 py-2 text-sm text-white placeholder-stone-500 focus:border-ocean-500 focus:outline-none"
            />
            {urlError && <p className="text-xs text-coral-400">{urlError}</p>}
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
    clearCoreRpcUrlCache();
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
