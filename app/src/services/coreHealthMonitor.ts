/**
 * coreHealthMonitor — polls the local Rust sidecar's `openhuman.connectivity_diag`
 * endpoint and dispatches `setCore` to the connectivitySlice (#1527).
 *
 * Polling cadence is adaptive:
 *   - healthy   : 30s (cheap heartbeat)
 *   - degraded  : 5s (fast recovery detection)
 *
 * A single transient failure is not enough to flip the channel — we require
 * `FAIL_THRESHOLD` consecutive failures to mark `unreachable` so a single
 * dropped TCP packet doesn't pop a scary blocking screen.
 */
import { setCore } from '../store/connectivitySlice';
import { store } from '../store/index';
import { callCoreRpc } from './coreRpcClient';

const HEALTHY_INTERVAL_MS = 30_000;
const DEGRADED_INTERVAL_MS = 5_000;
const FAIL_THRESHOLD = 2;

let timer: ReturnType<typeof setTimeout> | null = null;
let consecutiveFails = 0;
let stopped = true;

async function probe(): Promise<void> {
  try {
    await callCoreRpc({ method: 'openhuman.connectivity_diag', params: {} });
    consecutiveFails = 0;
    store.dispatch(setCore({ value: 'reachable' }));
  } catch (err) {
    consecutiveFails += 1;
    const message = err instanceof Error ? err.message : String(err);
    if (consecutiveFails >= FAIL_THRESHOLD) {
      store.dispatch(setCore({ value: 'unreachable', error: message }));
    }
  } finally {
    if (!stopped) schedule();
  }
}

function schedule(): void {
  if (timer != null) clearTimeout(timer);
  // Use the failure streak (not just the Redux state) so we enter degraded
  // 5s polling on the *first* miss — before the threshold flips `core` to
  // `unreachable`. Without this, first-failure retries stayed at 30s.
  // (addresses @coderabbitai on coreHealthMonitor.ts:46)
  const state = store.getState().connectivity.core;
  const isDegraded = consecutiveFails > 0 || state !== 'reachable';
  const interval = isDegraded ? DEGRADED_INTERVAL_MS : HEALTHY_INTERVAL_MS;
  timer = setTimeout(() => void probe(), interval);
}

export function startCoreHealthMonitor(): void {
  if (!stopped) return;
  stopped = false;
  consecutiveFails = 0;
  void probe();
}

export function stopCoreHealthMonitor(): void {
  stopped = true;
  if (timer != null) {
    clearTimeout(timer);
    timer = null;
  }
}
