import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  capabilityForModel,
  downloadAsset,
  fetchInstalledAssets,
  fetchInstalledModels,
  fetchLocalAiStatus,
  fetchPresets,
  formatBytes,
  getMemoryTreeLlm,
  type ModelDescriptor,
  setMemoryTreeLlm,
} from '../settingsApi';

// Stub the underlying tauri-command wrappers; we're testing the
// camelCase→snake_case translation + simple try/catch shells, not the
// RPC plumbing.
vi.mock('../../../utils/tauriCommands', () => ({
  isTauri: vi.fn(() => true),
  memoryTreeGetLlm: vi.fn(),
  memoryTreeSetLlm: vi.fn(),
  openhumanLocalAiAssetsStatus: vi.fn(),
  openhumanLocalAiStatus: vi.fn(),
  openhumanLocalAiDiagnostics: vi.fn(),
  openhumanLocalAiPresets: vi.fn(),
  openhumanLocalAiDownloadAsset: vi.fn(),
}));

const tauri = (await import('../../../utils/tauriCommands')) as unknown as {
  memoryTreeGetLlm: ReturnType<typeof vi.fn>;
  memoryTreeSetLlm: ReturnType<typeof vi.fn>;
  openhumanLocalAiAssetsStatus: ReturnType<typeof vi.fn>;
  openhumanLocalAiStatus: ReturnType<typeof vi.fn>;
  openhumanLocalAiDiagnostics: ReturnType<typeof vi.fn>;
  openhumanLocalAiPresets: ReturnType<typeof vi.fn>;
  openhumanLocalAiDownloadAsset: ReturnType<typeof vi.fn>;
};

beforeEach(() => {
  Object.values(tauri).forEach(fn => fn.mockReset());
});

describe('getMemoryTreeLlm', () => {
  it('returns the current backend value from the RPC', async () => {
    tauri.memoryTreeGetLlm.mockResolvedValueOnce({ current: 'cloud' });
    await expect(getMemoryTreeLlm()).resolves.toBe('cloud');
    expect(tauri.memoryTreeGetLlm).toHaveBeenCalledTimes(1);
  });
});

describe('setMemoryTreeLlm', () => {
  it('passes only the backend when no options are supplied', async () => {
    tauri.memoryTreeSetLlm.mockResolvedValueOnce({ current: 'cloud' });
    await setMemoryTreeLlm('cloud');
    expect(tauri.memoryTreeSetLlm).toHaveBeenCalledWith({ backend: 'cloud' });
  });

  it('translates camelCase options to snake_case wire fields and passes only those that are set', async () => {
    tauri.memoryTreeSetLlm.mockResolvedValueOnce({ current: 'local' });
    await setMemoryTreeLlm('local', { extractModel: 'a:b', summariserModel: 'c:d' });
    expect(tauri.memoryTreeSetLlm).toHaveBeenCalledWith({
      backend: 'local',
      extract_model: 'a:b',
      summariser_model: 'c:d',
    });
    // cloudModel was unset → cloud_model must NOT be on the wire payload.
    expect(tauri.memoryTreeSetLlm.mock.calls[0][0]).not.toHaveProperty('cloud_model');
  });

  it('returns the effective backend value the core decided on', async () => {
    tauri.memoryTreeSetLlm.mockResolvedValueOnce({ current: 'cloud' });
    const out = await setMemoryTreeLlm('local');
    expect(out).toEqual({ effective: 'cloud' });
  });
});

describe('fetchInstalledAssets', () => {
  it('unwraps the `result` field on success', async () => {
    tauri.openhumanLocalAiAssetsStatus.mockResolvedValueOnce({ result: { foo: 1 } });
    await expect(fetchInstalledAssets()).resolves.toEqual({ foo: 1 });
  });
  it('swallows RPC errors and returns null', async () => {
    tauri.openhumanLocalAiAssetsStatus.mockRejectedValueOnce(new Error('boom'));
    await expect(fetchInstalledAssets()).resolves.toBeNull();
  });
});

describe('fetchLocalAiStatus', () => {
  it('returns null on RPC failure', async () => {
    tauri.openhumanLocalAiStatus.mockRejectedValueOnce(new Error('nope'));
    await expect(fetchLocalAiStatus()).resolves.toBeNull();
  });
});

describe('fetchInstalledModels', () => {
  it('returns the installed_models array', async () => {
    tauri.openhumanLocalAiDiagnostics.mockResolvedValueOnce({
      installed_models: [{ name: 'bge-m3', size_bytes: 1 }],
    });
    const got = await fetchInstalledModels();
    expect(got).toHaveLength(1);
    expect(got[0]?.name).toBe('bge-m3');
  });
  it('returns [] when the RPC rejects', async () => {
    tauri.openhumanLocalAiDiagnostics.mockRejectedValueOnce(new Error('rpc down'));
    await expect(fetchInstalledModels()).resolves.toEqual([]);
  });
  it('returns [] when installed_models is missing', async () => {
    tauri.openhumanLocalAiDiagnostics.mockResolvedValueOnce({});
    await expect(fetchInstalledModels()).resolves.toEqual([]);
  });
});

describe('fetchPresets', () => {
  it('forwards the response on success', async () => {
    tauri.openhumanLocalAiPresets.mockResolvedValueOnce({ presets: [] });
    await expect(fetchPresets()).resolves.toEqual({ presets: [] });
  });
  it('returns null on failure', async () => {
    tauri.openhumanLocalAiPresets.mockRejectedValueOnce(new Error('x'));
    await expect(fetchPresets()).resolves.toBeNull();
  });
});

describe('downloadAsset', () => {
  it('returns the result envelope on success', async () => {
    tauri.openhumanLocalAiDownloadAsset.mockResolvedValueOnce({ result: { ok: true } });
    await expect(downloadAsset('chat')).resolves.toEqual({ ok: true });
    expect(tauri.openhumanLocalAiDownloadAsset).toHaveBeenCalledWith('chat');
  });
  it('returns null on failure (and does not throw)', async () => {
    tauri.openhumanLocalAiDownloadAsset.mockRejectedValueOnce(new Error('disconnected'));
    await expect(downloadAsset('embedding')).resolves.toBeNull();
  });
});

describe('capabilityForModel', () => {
  const make = (roles: ModelDescriptor['roles']): ModelDescriptor => ({
    id: 'x',
    size: '0',
    approxBytes: 0,
    ramHint: '0',
    category: 'fast',
    note: '',
    roles,
  });
  it('maps embedder → embedding', () => {
    expect(capabilityForModel(make(['embedder']))).toBe('embedding');
  });
  it('maps extract / summariser → chat', () => {
    expect(capabilityForModel(make(['extract']))).toBe('chat');
    expect(capabilityForModel(make(['summariser']))).toBe('chat');
    expect(capabilityForModel(make(['extract', 'summariser']))).toBe('chat');
  });
  it('returns null when no role binds to a known capability', () => {
    expect(capabilityForModel(make([]))).toBeNull();
  });
});

describe('formatBytes', () => {
  it('falls back to em-dash for non-finite or zero', () => {
    expect(formatBytes(Number.NaN)).toBe('—');
    expect(formatBytes(0)).toBe('—');
    expect(formatBytes(Number.POSITIVE_INFINITY)).toBe('—');
  });
  it('formats GB for >= 1 GiB', () => {
    expect(formatBytes(2.5 * 1024 ** 3)).toBe('2.5 GB');
  });
  it('formats MB (rounded) for sub-GB inputs', () => {
    expect(formatBytes(150 * 1024 ** 2)).toBe('150 MB');
    expect(formatBytes(1024 ** 2 + 1)).toBe('1 MB');
  });
});
