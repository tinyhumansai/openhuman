import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  readinessCheckAll,
  readinessOllamaModels,
  readinessValidateModelConnection,
} from './readiness';

const callCoreRpcMock = vi.fn();
let tauri = true;

vi.mock('../../services/coreRpcClient', () => ({
  callCoreRpc: (args: unknown) => callCoreRpcMock(args),
}));

vi.mock('./common', () => ({ isTauri: () => tauri }));

describe('readiness tauri commands', () => {
  beforeEach(() => {
    callCoreRpcMock.mockReset();
    tauri = true;
  });

  it('checkAll calls the correct method and returns the bare report', async () => {
    const report = { checks: [], overall: 'ok', model_connection_ok: true, host_os: 'macos' };
    callCoreRpcMock.mockResolvedValue(report);
    const result = await readinessCheckAll();
    expect(callCoreRpcMock).toHaveBeenCalledWith({ method: 'openhuman.readiness_check_all' });
    expect(result).toEqual(report);
  });

  it('unwraps the { result, logs } envelope when core attaches logs', async () => {
    const report = { checks: [], overall: 'warn', model_connection_ok: false, host_os: 'linux' };
    callCoreRpcMock.mockResolvedValue({ result: report, logs: ['readiness check completed'] });
    const result = await readinessCheckAll();
    expect(result).toEqual(report);
  });

  it('validateModelConnection targets its own method', async () => {
    callCoreRpcMock.mockResolvedValue({ ok: true, provider_id: 'anthropic' });
    const result = await readinessValidateModelConnection();
    expect(callCoreRpcMock).toHaveBeenCalledWith({
      method: 'openhuman.readiness_validate_model_connection',
    });
    expect(result.ok).toBe(true);
  });

  it('ollamaModels targets its own method', async () => {
    callCoreRpcMock.mockResolvedValue({ reachable: true, models: ['llama3'] });
    const result = await readinessOllamaModels();
    expect(callCoreRpcMock).toHaveBeenCalledWith({ method: 'openhuman.readiness_ollama_models' });
    expect(result.models).toEqual(['llama3']);
  });

  it('throws when not running in Tauri', async () => {
    tauri = false;
    await expect(readinessCheckAll()).rejects.toThrow('Not running in Tauri');
    expect(callCoreRpcMock).not.toHaveBeenCalled();
  });
});
