import { beforeEach, describe, expect, it, vi } from 'vitest';

// Importing AFTER mocks are registered so the service binds to the stubs.
import { downloadArtifact, revealArtifactInFileManager } from '../artifactDownloadService';

// Mock the Tauri common shim (safeInvoke + isTauri) so the service
// runs entirely in JS — no real Tauri IPC reach.
const invokeMock = vi.fn();
const isTauriMock = vi.fn(() => true);
vi.mock('../../utils/tauriCommands/common', () => ({
  safeInvoke: (...args: unknown[]) => invokeMock(...args),
  isTauri: () => isTauriMock(),
}));

// Mock the core RPC client — the service only uses one method.
const callCoreRpcMock = vi.fn();
vi.mock('../coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => callCoreRpcMock(...args),
}));

// Mock the opener plugin's reveal binding so we can assert it is
// invoked with the canonical absolute-path argument.
const revealMock = vi.fn();
vi.mock('@tauri-apps/plugin-opener', () => ({
  revealItemInDir: (...args: unknown[]) => revealMock(...args),
}));

beforeEach(() => {
  invokeMock.mockReset();
  callCoreRpcMock.mockReset();
  revealMock.mockReset();
  isTauriMock.mockReset();
  isTauriMock.mockReturnValue(true);
});

describe('downloadArtifact', () => {
  it('refuses outside Tauri with a user-facing message', async () => {
    isTauriMock.mockReturnValueOnce(false);
    const out = await downloadArtifact('a-1', 'Deck', 'pptx');
    expect(out).toEqual({ ok: false, error: 'Downloads are only available in the desktop app' });
    expect(callCoreRpcMock).not.toHaveBeenCalled();
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it('refuses an empty artifact id', async () => {
    const out = await downloadArtifact('   ', 'Deck', 'pptx');
    expect(out).toEqual({ ok: false, error: 'artifact id missing' });
    expect(callCoreRpcMock).not.toHaveBeenCalled();
  });

  it('surfaces a friendly error when ai_get_artifact rejects (Error)', async () => {
    callCoreRpcMock.mockRejectedValueOnce(new Error('rpc dropped'));
    const out = await downloadArtifact('a-1', 'Deck', 'pptx');
    expect(out.ok).toBe(false);
    expect(out.error).toBe('failed to resolve artifact: rpc dropped');
  });

  it('stringifies non-Error rejections from the core RPC', async () => {
    callCoreRpcMock.mockRejectedValueOnce('boom');
    const out = await downloadArtifact('a-1', 'Deck', 'pptx');
    expect(out.ok).toBe(false);
    expect(out.error).toBe('failed to resolve artifact: boom');
  });

  it('returns a clear error when the core response lacks absolute_path', async () => {
    callCoreRpcMock.mockResolvedValueOnce({ meta: { title: 'Deck' } });
    const out = await downloadArtifact('a-1', 'Deck', 'pptx');
    expect(out).toEqual({ ok: false, error: 'artifact path missing from core response' });
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it('treats a null core response as missing path', async () => {
    callCoreRpcMock.mockResolvedValueOnce(null);
    const out = await downloadArtifact('a-1', 'Deck', 'pptx');
    expect(out).toEqual({ ok: false, error: 'artifact path missing from core response' });
  });

  it('prefers the persisted title over the caller fallback', async () => {
    callCoreRpcMock.mockResolvedValueOnce({
      absolute_path: '/workspace/artifacts/a-1/deck.pptx',
      meta: { title: 'Real Title' },
    });
    invokeMock.mockResolvedValueOnce('/Users/me/Downloads/Real Title.pptx');

    const out = await downloadArtifact('a-1', 'Caller Fallback', 'pptx');

    expect(out).toEqual({ ok: true, path: '/Users/me/Downloads/Real Title.pptx' });
    expect(callCoreRpcMock).toHaveBeenCalledWith({
      method: 'openhuman.ai_get_artifact',
      params: { artifact_id: 'a-1' },
    });
    expect(invokeMock).toHaveBeenCalledWith('download_artifact_to_downloads', {
      sourcePath: '/workspace/artifacts/a-1/deck.pptx',
      filename: 'Real Title.pptx',
    });
  });

  it('falls back to the caller-supplied title when meta.title is blank', async () => {
    callCoreRpcMock.mockResolvedValueOnce({
      absolute_path: '/workspace/artifacts/a-1/deck.pptx',
      meta: { title: '   ' },
    });
    invokeMock.mockResolvedValueOnce('/Users/me/Downloads/Deck.pptx');

    await downloadArtifact('a-1', 'Deck', 'pptx');
    expect(invokeMock).toHaveBeenCalledWith('download_artifact_to_downloads', {
      sourcePath: '/workspace/artifacts/a-1/deck.pptx',
      filename: 'Deck.pptx',
    });
  });

  it('uses the "artifact" placeholder when both title sources are empty', async () => {
    callCoreRpcMock.mockResolvedValueOnce({
      absolute_path: '/workspace/artifacts/a-1/x.bin',
      meta: {},
    });
    invokeMock.mockResolvedValueOnce('/Users/me/Downloads/artifact.bin');

    await downloadArtifact('a-1', '   ', 'bin');
    expect(invokeMock).toHaveBeenCalledWith('download_artifact_to_downloads', {
      sourcePath: '/workspace/artifacts/a-1/x.bin',
      filename: 'artifact.bin',
    });
  });

  it('strips leading dots from the extension before appending', async () => {
    callCoreRpcMock.mockResolvedValueOnce({
      absolute_path: '/workspace/artifacts/a-1/deck.pptx',
      meta: { title: 'Deck' },
    });
    invokeMock.mockResolvedValueOnce('/Users/me/Downloads/Deck.pptx');

    await downloadArtifact('a-1', 'Deck', '..pptx');
    expect(invokeMock).toHaveBeenCalledWith('download_artifact_to_downloads', {
      sourcePath: '/workspace/artifacts/a-1/deck.pptx',
      filename: 'Deck.pptx',
    });
  });

  it('omits the dot when extension is empty', async () => {
    callCoreRpcMock.mockResolvedValueOnce({
      absolute_path: '/workspace/artifacts/a-1/raw',
      meta: { title: 'Raw' },
    });
    invokeMock.mockResolvedValueOnce('/Users/me/Downloads/Raw');

    await downloadArtifact('a-1', 'Raw', '   ');
    expect(invokeMock).toHaveBeenCalledWith('download_artifact_to_downloads', {
      sourcePath: '/workspace/artifacts/a-1/raw',
      filename: 'Raw',
    });
  });

  it('returns the Tauri error when the copy command rejects', async () => {
    callCoreRpcMock.mockResolvedValueOnce({
      absolute_path: '/workspace/artifacts/a-1/deck.pptx',
      meta: { title: 'Deck' },
    });
    invokeMock.mockRejectedValueOnce(new Error('disk full'));

    const out = await downloadArtifact('a-1', 'Deck', 'pptx');
    expect(out).toEqual({ ok: false, error: 'disk full' });
  });

  it('stringifies a non-Error Tauri rejection', async () => {
    callCoreRpcMock.mockResolvedValueOnce({
      absolute_path: '/workspace/artifacts/a-1/deck.pptx',
      meta: { title: 'Deck' },
    });
    invokeMock.mockRejectedValueOnce('nope');

    const out = await downloadArtifact('a-1', 'Deck', 'pptx');
    expect(out).toEqual({ ok: false, error: 'nope' });
  });
});

describe('revealArtifactInFileManager', () => {
  it('no-ops outside Tauri', async () => {
    isTauriMock.mockReturnValueOnce(false);
    const ok = await revealArtifactInFileManager('/Users/me/Downloads/Deck.pptx');
    expect(ok).toBe(false);
    expect(revealMock).not.toHaveBeenCalled();
  });

  it('no-ops on an empty absolute path', async () => {
    const ok = await revealArtifactInFileManager('   ');
    expect(ok).toBe(false);
    expect(revealMock).not.toHaveBeenCalled();
  });

  it('routes through the typed plugin binding', async () => {
    revealMock.mockResolvedValueOnce(undefined);
    const ok = await revealArtifactInFileManager('/Users/me/Downloads/Deck.pptx');
    expect(ok).toBe(true);
    expect(revealMock).toHaveBeenCalledWith('/Users/me/Downloads/Deck.pptx');
  });

  it('swallows reveal failures and returns false', async () => {
    revealMock.mockRejectedValueOnce(new Error('opener missing'));
    const ok = await revealArtifactInFileManager('/Users/me/Downloads/Deck.pptx');
    expect(ok).toBe(false);
  });
});
