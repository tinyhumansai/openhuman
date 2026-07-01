/**
 * First-run readiness aggregator commands (issue #4252).
 *
 * Thin typed wrappers over the core `readiness.*` JSON-RPC controllers used by
 * the onboarding "Readiness" step and the Settings diagnostics surface.
 */
import { callCoreRpc } from '../../services/coreRpcClient';
import { isTauri } from './common';

export type ReadinessStatus = 'ok' | 'warn' | 'fail' | 'skipped';

export interface ReadinessCheck {
  id: string;
  category: string;
  status: ReadinessStatus;
  title: string;
  detail: string;
  /** Specific recovery action when not `ok`. Absent when `ok`. */
  remediation?: string;
  platform: string;
  required: boolean;
}

export interface ReadinessReport {
  checks: ReadinessCheck[];
  overall: ReadinessStatus;
  /** Whether at least one model connection validated — the setup Finish gate. */
  model_connection_ok: boolean;
  host_os: string;
}

export interface ModelConnProbe {
  ok: boolean;
  provider_id: string;
  error?: string;
}

export interface OllamaProbe {
  reachable: boolean;
  models: string[];
}

/** Core controllers that attach CLI logs return `{ result, logs }`; unwrap it. */
function unwrapLogged<T>(raw: unknown): T {
  if (
    raw &&
    typeof raw === 'object' &&
    'result' in (raw as Record<string, unknown>) &&
    'logs' in (raw as Record<string, unknown>)
  ) {
    return (raw as { result: T }).result;
  }
  return raw as T;
}

export async function readinessCheckAll(): Promise<ReadinessReport> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  const raw = await callCoreRpc<unknown>({ method: 'openhuman.readiness_check_all' });
  return unwrapLogged<ReadinessReport>(raw);
}

export async function readinessValidateModelConnection(): Promise<ModelConnProbe> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  const raw = await callCoreRpc<unknown>({
    method: 'openhuman.readiness_validate_model_connection',
  });
  return unwrapLogged<ModelConnProbe>(raw);
}

export async function readinessOllamaModels(): Promise<OllamaProbe> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  const raw = await callCoreRpc<unknown>({ method: 'openhuman.readiness_ollama_models' });
  return unwrapLogged<OllamaProbe>(raw);
}
