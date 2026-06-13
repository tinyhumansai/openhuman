/**
 * Service-backed transport for the boot-check orchestrator.
 *
 * The orchestrator (`app/src/lib/bootCheck/`) keeps all I/O behind a
 * `BootCheckTransport` interface so it can be unit-tested without Tauri.
 * This module is the production implementation: it owns the direct
 * `invoke` and `callCoreRpc` references so IPC stays localized to
 * `app/src/services/` per project conventions.
 */
import { invoke } from '@tauri-apps/api/core';

import type { BootCheckTransport } from '../lib/bootCheck';
import { callCoreRpc } from './coreRpcClient';

async function callRpc<T>(method: string, params?: Record<string, unknown>): Promise<T> {
  return callCoreRpc<T>({ method, params });
}

async function invokeCmd<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return invoke<T>(cmd, args);
}

/**
 * Invoke the `recover_port_conflict` Tauri command to reap stale OpenHuman
 * processes and restart the embedded core on any available port.
 */
export async function recoverPortConflict(): Promise<{
  success: boolean;
  message: string;
  new_port?: number;
}> {
  return invokeCmd('recover_port_conflict');
}

export const bootCheckTransport: BootCheckTransport = { callRpc, invokeCmd, recoverPortConflict };
