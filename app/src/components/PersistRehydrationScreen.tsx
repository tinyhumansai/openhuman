import debug from 'debug';
import { useEffect, useState } from 'react';

import { persistor } from '../store';
import RouteLoadingScreen from './RouteLoadingScreen';

const persistWarn = debug('persist:warn');

/**
 * If rehydration has not completed by this cap we surface a recovery CTA.
 * Chosen to be long enough that slow disks / antivirus scans don't flap
 * users into it, but short enough that a stuck splash screen is noticeable.
 */
const REHYDRATION_WARN_TIMEOUT_MS = 10_000;

/**
 * Loading surface used as the `loading` prop for `<PersistGate>`.
 *
 * PersistGate alone has no deadline: if rehydration stalls (corrupt
 * `localStorage`, disk stalls, a storage adapter that never resolves) the
 * user sees a permanent splash with no way out. After `REHYDRATION_WARN_TIMEOUT_MS`
 * we swap in a recovery panel that lets the user purge persisted state and
 * reload. PersistGate still tears down this component the moment rehydration
 * finishes, so a slow-but-eventual boot behaves identically to today.
 */
function PersistRehydrationScreen() {
  const [timedOut, setTimedOut] = useState(false);
  const [resetting, setResetting] = useState(false);

  useEffect(() => {
    const timer = setTimeout(() => {
      persistWarn(
        'redux-persist rehydration exceeded %dms — surfacing recovery CTA',
        REHYDRATION_WARN_TIMEOUT_MS
      );
      setTimedOut(true);
    }, REHYDRATION_WARN_TIMEOUT_MS);
    return () => clearTimeout(timer);
  }, []);

  if (!timedOut) return <RouteLoadingScreen />;

  const handleReset = async () => {
    if (resetting) return;
    setResetting(true);
    try {
      await persistor.purge();
    } catch (err) {
      persistWarn('persistor.purge() failed: %o', err);
    }
    window.location.reload();
  };

  return (
    <div className="fixed inset-0 flex items-center justify-center bg-canvas-50 p-6">
      <div className="max-w-sm w-full space-y-4 rounded-xl border border-stone-200 bg-white p-6 shadow-soft text-center">
        <p className="text-sm font-medium text-stone-900">
          OpenHuman is taking longer than usual to load.
        </p>
        <p className="text-xs text-stone-500 leading-relaxed">
          Local app state may be corrupted. Resetting will sign you out and clear cached data on
          this device; you&apos;ll reconnect your accounts on next launch. Your data on the server
          is not affected.
        </p>
        <button
          type="button"
          onClick={handleReset}
          disabled={resetting}
          className="w-full rounded-xl bg-primary-500 px-4 py-2.5 text-sm font-medium text-white transition-colors hover:bg-primary-600 disabled:cursor-not-allowed disabled:opacity-60">
          {resetting ? 'Resetting…' : 'Reset local state'}
        </button>
      </div>
    </div>
  );
}

export default PersistRehydrationScreen;
