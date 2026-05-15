/**
 * Wires `navigator.onLine` + `online`/`offline` events to the
 * connectivitySlice so the UI reflects the real device internet state
 * (#1527).
 *
 * Called once at app boot from `App.tsx`. Idempotent — repeat invocations
 * no-op via `started`.
 */
import { setInternet } from '../store/connectivitySlice';
import { store } from '../store/index';

let started = false;

function snapshot(): void {
  const online = typeof navigator !== 'undefined' ? navigator.onLine !== false : true;
  store.dispatch(setInternet({ value: online ? 'online' : 'offline' }));
}

export function startInternetStatusListener(): void {
  if (started) return;
  started = true;
  if (typeof window === 'undefined') return;

  snapshot();
  window.addEventListener('online', snapshot);
  window.addEventListener('offline', snapshot);
}
