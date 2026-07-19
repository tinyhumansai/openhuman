import type { HaltState } from '../../store/safetySlice';
import { callCoreRpc } from '../coreRpcClient';

/** Normalize the CLI envelope `{ result, logs }` and bare-value shapes. */
const unwrapValue = <T>(raw: unknown): T => {
  if (raw && typeof raw === 'object' && 'result' in (raw as Record<string, unknown>)) {
    return (raw as { result: T }).result;
  }
  return raw as T;
};

export async function emergencyStop(reason?: string): Promise<HaltState> {
  console.debug('[emergency] rpc → openhuman.emergency_stop', { reason: reason ?? 'none' });
  const raw = await callCoreRpc<unknown>({
    method: 'openhuman.emergency_stop',
    params: reason ? { reason } : {},
  });
  return unwrapValue<HaltState>(raw);
}

export async function emergencyResume(): Promise<HaltState> {
  console.debug('[emergency] rpc → openhuman.emergency_resume');
  const raw = await callCoreRpc<unknown>({ method: 'openhuman.emergency_resume', params: {} });
  return unwrapValue<HaltState>(raw);
}

export async function emergencyStatus(): Promise<HaltState> {
  console.debug('[emergency] rpc → openhuman.emergency_status');
  const raw = await callCoreRpc<unknown>({ method: 'openhuman.emergency_status', params: {} });
  return unwrapValue<HaltState>(raw);
}
