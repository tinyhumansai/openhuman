/**
 * Harness Init Service
 *
 * Thin RPC wrappers around the core `harness_init` controller plus parsing into
 * typed snapshots. The initialization overlay polls `fetchHarnessInitStatus`
 * (read-only) and calls `runHarnessInit` to retry after a failure.
 *
 * Mirrors the polling contract used by `daemonHealthService`, but stays a plain
 * data layer — the React overlay owns the interval and visibility.
 */
import { callCoreRpc } from './coreRpcClient';

export type HarnessInitStepState = 'pending' | 'running' | 'done' | 'failed' | 'skipped';
export type HarnessInitOverall = 'idle' | 'running' | 'done' | 'failed';

export interface HarnessInitStep {
  id: string;
  label: string;
  required: boolean;
  state: HarnessInitStepState;
  message: string | null;
  percent: number | null;
  updatedAt: string | null;
}

export interface HarnessInitSnapshot {
  overall: HarnessInitOverall;
  steps: HarnessInitStep[];
  startedAt: string | null;
  finishedAt: string | null;
}

interface RawSnapshotEnvelope {
  snapshot?: unknown;
}

const STEP_STATES: HarnessInitStepState[] = ['pending', 'running', 'done', 'failed', 'skipped'];
const OVERALL_STATES: HarnessInitOverall[] = ['idle', 'running', 'done', 'failed'];

function asString(value: unknown): string | null {
  return typeof value === 'string' ? value : null;
}

function parseStep(raw: unknown): HarnessInitStep | null {
  if (!raw || typeof raw !== 'object') {
    return null;
  }
  const data = raw as Record<string, unknown>;
  const id = asString(data.id);
  const state = asString(data.state) as HarnessInitStepState | null;
  if (!id || !state || !STEP_STATES.includes(state)) {
    return null;
  }
  return {
    id,
    label: asString(data.label) ?? id,
    required: data.required === true,
    state,
    message: asString(data.message),
    percent: typeof data.percent === 'number' ? data.percent : null,
    updatedAt: asString(data.updated_at),
  };
}

/** Parse the `{ snapshot: {...} }` RPC result envelope into a typed snapshot. */
export function parseHarnessInitSnapshot(payload: unknown): HarnessInitSnapshot | null {
  const envelope = (payload ?? {}) as RawSnapshotEnvelope;
  const raw = envelope.snapshot;
  if (!raw || typeof raw !== 'object') {
    return null;
  }
  const data = raw as Record<string, unknown>;
  const overall = asString(data.overall) as HarnessInitOverall | null;
  if (!overall || !OVERALL_STATES.includes(overall)) {
    return null;
  }
  const stepsRaw = Array.isArray(data.steps) ? data.steps : [];
  const steps = stepsRaw.map(parseStep).filter((s): s is HarnessInitStep => s !== null);
  return {
    overall,
    steps,
    startedAt: asString(data.started_at),
    finishedAt: asString(data.finished_at),
  };
}

/** Read the current init progress. Read-only — never triggers provisioning. */
export async function fetchHarnessInitStatus(): Promise<HarnessInitSnapshot | null> {
  const payload = await callCoreRpc<unknown>({ method: 'openhuman.harness_init_status' });
  return parseHarnessInitSnapshot(payload);
}

/** Re-run init (retry). `force` re-runs even already-satisfied steps. */
export async function runHarnessInit(force = false): Promise<HarnessInitSnapshot | null> {
  const payload = await callCoreRpc<unknown>({
    method: 'openhuman.harness_init_run',
    params: { force },
    // Provisioning can download Python / Node / spaCy — allow a long budget.
    timeoutMs: 10 * 60 * 1000,
  });
  return parseHarnessInitSnapshot(payload);
}
