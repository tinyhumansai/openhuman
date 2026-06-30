/**
 * artifactDownloadService coverage (#3024).
 *
 * Exercises every branch of `downloadArtifact`, `deleteArtifact`, and
 * `revealArtifactInFileManager` — including the typed error-code
 * payloads, the title double-extension guard, and the non-Tauri /
 * empty-id / RPC-error / invoke-error paths.
 *
 * Mocks: `safeInvoke` + `isTauri` from `utils/tauriCommands/common`,
 * `callCoreRpc` from `services/coreRpcClient`, and `revealItemInDir`
 * from `@tauri-apps/plugin-opener`. The service has no other I/O.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  deleteArtifact,
  downloadArtifact,
  revealArtifactInFileManager,
  saveArtifactViaDialog,
} from '../artifactDownloadService';
import { callCoreRpc } from '../coreRpcClient';

const hoisted = vi.hoisted(() => ({
  invoke: vi.fn(),
  isTauri: vi.fn(() => true),
  revealItemInDir: vi.fn(),
}));

vi.mock('../../utils/tauriCommands/common', () => ({
  isTauri: hoisted.isTauri,
  safeInvoke: (...args: unknown[]) => hoisted.invoke(...args),
}));

vi.mock('../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

vi.mock('@tauri-apps/plugin-opener', () => ({
  revealItemInDir: (...args: unknown[]) => hoisted.revealItemInDir(...args),
}));

describe('downloadArtifact', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    hoisted.isTauri.mockReturnValue(true);
  });

  it('returns NOT_DESKTOP code when called outside Tauri', async () => {
    hoisted.isTauri.mockReturnValueOnce(false);
    const outcome = await downloadArtifact('art-1', 'Deck', 'pptx');
    expect(outcome).toEqual({
      ok: false,
      code: 'NOT_DESKTOP',
      error: expect.stringContaining('desktop'),
    });
    expect(callCoreRpc).not.toHaveBeenCalled();
    expect(hoisted.invoke).not.toHaveBeenCalled();
  });

  it('returns MISSING_ARTIFACT_ID for empty / whitespace ids', async () => {
    const outcome = await downloadArtifact('   ', 'Deck', 'pptx');
    expect(outcome.ok).toBe(false);
    expect(outcome.code).toBe('MISSING_ARTIFACT_ID');
    expect(callCoreRpc).not.toHaveBeenCalled();
  });

  it('returns RESOLVE_FAILED when the core RPC throws (Error instance)', async () => {
    vi.mocked(callCoreRpc).mockRejectedValueOnce(new Error('boom'));
    const outcome = await downloadArtifact('art-1', 'Deck', 'pptx');
    expect(outcome).toEqual({ ok: false, code: 'RESOLVE_FAILED', error: 'boom' });
  });

  it('returns RESOLVE_FAILED with String coercion when RPC throws a non-Error', async () => {
    vi.mocked(callCoreRpc).mockRejectedValueOnce('plain-string-rejection');
    const outcome = await downloadArtifact('art-1', 'Deck', 'pptx');
    expect(outcome.code).toBe('RESOLVE_FAILED');
    expect(outcome.error).toBe('plain-string-rejection');
  });

  it('returns MISSING_ARTIFACT_PATH when core resolves without absolute_path', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ meta: { id: 'art-1', title: 'Deck' } });
    const outcome = await downloadArtifact('art-1', 'Deck', 'pptx');
    expect(outcome.code).toBe('MISSING_ARTIFACT_PATH');
  });

  it('returns MISSING_ARTIFACT_PATH when core returns nullish payload', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce(null);
    const outcome = await downloadArtifact('art-1', 'Deck', 'pptx');
    expect(outcome.code).toBe('MISSING_ARTIFACT_PATH');
  });

  it('happy path: resolves → invokes download → returns ok + dest path', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      absolute_path: '/workspace/artifacts/art-1/deck.pptx',
      meta: { id: 'art-1', title: 'climate-deck' },
    });
    hoisted.invoke.mockResolvedValueOnce('/Users/me/Downloads/climate-deck.pptx');
    const outcome = await downloadArtifact('art-1', 'fallback-title', 'pptx');
    expect(outcome).toEqual({ ok: true, path: '/Users/me/Downloads/climate-deck.pptx' });
    expect(hoisted.invoke).toHaveBeenCalledWith('download_artifact_to_downloads', {
      sourcePath: '/workspace/artifacts/art-1/deck.pptx',
      filename: 'climate-deck.pptx',
    });
  });

  it('falls back to the caller-supplied title when meta.title is blank', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      absolute_path: '/p/file',
      meta: { id: 'art-1', title: '   ' },
    });
    hoisted.invoke.mockResolvedValueOnce('/dest/file.pdf');
    await downloadArtifact('art-1', 'caller-fallback', 'pdf');
    expect(hoisted.invoke).toHaveBeenCalledWith('download_artifact_to_downloads', {
      sourcePath: '/p/file',
      filename: 'caller-fallback.pdf',
    });
  });

  it('falls back to "artifact" when both meta.title and fallbackTitle are blank', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ absolute_path: '/p/file' });
    hoisted.invoke.mockResolvedValueOnce('/dest/x');
    await downloadArtifact('art-1', '   ', 'bin');
    expect(hoisted.invoke).toHaveBeenCalledWith('download_artifact_to_downloads', {
      sourcePath: '/p/file',
      filename: 'artifact.bin',
    });
  });

  it('strips leading dots from the extension hint', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      absolute_path: '/p/file',
      meta: { title: 'deck' },
    });
    hoisted.invoke.mockResolvedValueOnce('/dest/deck.pptx');
    await downloadArtifact('art-1', 'deck', '...pptx');
    expect(hoisted.invoke).toHaveBeenCalledWith('download_artifact_to_downloads', {
      sourcePath: '/p/file',
      filename: 'deck.pptx',
    });
  });

  it('does NOT double-append the extension when the title already carries it (same ext)', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      absolute_path: '/p/file',
      meta: { title: 'deck.pptx' },
    });
    hoisted.invoke.mockResolvedValueOnce('/dest/deck.pptx');
    await downloadArtifact('art-1', 'deck.pptx', 'pptx');
    expect(hoisted.invoke).toHaveBeenCalledWith('download_artifact_to_downloads', {
      sourcePath: '/p/file',
      filename: 'deck.pptx',
    });
  });

  it('does NOT double-append the extension (case-insensitive match on existing ext)', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      absolute_path: '/p/file',
      meta: { title: 'DECK.PPTX' },
    });
    hoisted.invoke.mockResolvedValueOnce('/dest/x');
    await downloadArtifact('art-1', 'DECK.PPTX', 'pptx');
    expect(hoisted.invoke).toHaveBeenCalledWith('download_artifact_to_downloads', {
      sourcePath: '/p/file',
      filename: 'DECK.PPTX',
    });
  });

  it('does NOT double-append when the title already has any trailing extension', async () => {
    // Title says .pdf but caller asked for pptx — defensive: keep the
    // user-visible title untouched rather than synthesising deck.pdf.pptx.
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      absolute_path: '/p/file',
      meta: { title: 'deck.pdf' },
    });
    hoisted.invoke.mockResolvedValueOnce('/dest/x');
    await downloadArtifact('art-1', 'deck.pdf', 'pptx');
    expect(hoisted.invoke).toHaveBeenCalledWith('download_artifact_to_downloads', {
      sourcePath: '/p/file',
      filename: 'deck.pdf',
    });
  });

  it('appends the extension when the title has no extension', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      absolute_path: '/p/file',
      meta: { title: 'climate-overview' },
    });
    hoisted.invoke.mockResolvedValueOnce('/dest/climate-overview.pptx');
    await downloadArtifact('art-1', 'climate-overview', 'pptx');
    expect(hoisted.invoke).toHaveBeenCalledWith('download_artifact_to_downloads', {
      sourcePath: '/p/file',
      filename: 'climate-overview.pptx',
    });
  });

  it('keeps the title bare when the extension hint is empty', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      absolute_path: '/p/file',
      meta: { title: 'just-a-title' },
    });
    hoisted.invoke.mockResolvedValueOnce('/dest/just-a-title');
    await downloadArtifact('art-1', 'just-a-title', '');
    expect(hoisted.invoke).toHaveBeenCalledWith('download_artifact_to_downloads', {
      sourcePath: '/p/file',
      filename: 'just-a-title',
    });
  });

  it('returns DOWNLOAD_FAILED when the Tauri invoke throws', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      absolute_path: '/p/file',
      meta: { title: 'deck' },
    });
    hoisted.invoke.mockRejectedValueOnce(new Error('disk full'));
    const outcome = await downloadArtifact('art-1', 'deck', 'pptx');
    expect(outcome).toEqual({ ok: false, code: 'DOWNLOAD_FAILED', error: 'disk full' });
  });

  it('returns DOWNLOAD_FAILED with String coercion when invoke rejects with a non-Error', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      absolute_path: '/p/file',
      meta: { title: 'deck' },
    });
    hoisted.invoke.mockRejectedValueOnce('not-an-Error');
    const outcome = await downloadArtifact('art-1', 'deck', 'pptx');
    expect(outcome).toEqual({ ok: false, code: 'DOWNLOAD_FAILED', error: 'not-an-Error' });
  });
});

describe('deleteArtifact', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('returns MISSING_ARTIFACT_ID for empty / whitespace ids without calling the RPC', async () => {
    const outcome = await deleteArtifact('   ');
    expect(outcome).toEqual({
      ok: false,
      code: 'MISSING_ARTIFACT_ID',
      error: 'artifact id missing',
    });
    expect(callCoreRpc).not.toHaveBeenCalled();
  });

  it('returns ok when the core RPC resolves', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce(null);
    const outcome = await deleteArtifact('art-1');
    expect(outcome).toEqual({ ok: true });
    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.ai_delete_artifact',
      params: { artifact_id: 'art-1' },
    });
  });

  it('returns DELETE_FAILED when the core RPC throws', async () => {
    vi.mocked(callCoreRpc).mockRejectedValueOnce(new Error('rpc down'));
    const outcome = await deleteArtifact('art-1');
    expect(outcome).toEqual({ ok: false, code: 'DELETE_FAILED', error: 'rpc down' });
  });

  it('returns DELETE_FAILED with String coercion on non-Error rejection', async () => {
    vi.mocked(callCoreRpc).mockRejectedValueOnce({ code: 500 });
    const outcome = await deleteArtifact('art-1');
    expect(outcome.code).toBe('DELETE_FAILED');
    expect(outcome.error).toBe('[object Object]');
  });
});

describe('revealArtifactInFileManager', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    hoisted.isTauri.mockReturnValue(true);
  });

  it('returns false when outside Tauri (no plugin call)', async () => {
    hoisted.isTauri.mockReturnValueOnce(false);
    const ok = await revealArtifactInFileManager('/some/path');
    expect(ok).toBe(false);
    expect(hoisted.revealItemInDir).not.toHaveBeenCalled();
  });

  it('returns false for an empty / whitespace path', async () => {
    const ok = await revealArtifactInFileManager('   ');
    expect(ok).toBe(false);
    expect(hoisted.revealItemInDir).not.toHaveBeenCalled();
  });

  it('delegates to revealItemInDir on success', async () => {
    hoisted.revealItemInDir.mockResolvedValueOnce(undefined);
    const ok = await revealArtifactInFileManager('/Users/me/Downloads/deck.pptx');
    expect(ok).toBe(true);
    expect(hoisted.revealItemInDir).toHaveBeenCalledWith('/Users/me/Downloads/deck.pptx');
  });

  it('swallows plugin errors and returns false (best-effort reveal)', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    hoisted.revealItemInDir.mockRejectedValueOnce(new Error('plugin failed'));
    const ok = await revealArtifactInFileManager('/Users/me/Downloads/deck.pptx');
    expect(ok).toBe(false);
    warn.mockRestore();
  });
});

describe('saveArtifactViaDialog (#3162)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    hoisted.isTauri.mockReturnValue(true);
  });

  const resolveOk = () =>
    vi
      .mocked(callCoreRpc)
      .mockResolvedValueOnce({
        absolute_path: '/ws/artifacts/a-1/deck.pptx',
        meta: { id: 'a-1', title: 'Deck' },
      } as never);

  it('returns NOT_DESKTOP outside Tauri', async () => {
    hoisted.isTauri.mockReturnValueOnce(false);
    const outcome = await saveArtifactViaDialog('a-1', 'Deck', 'pptx');
    expect(outcome).toEqual({
      ok: false,
      code: 'NOT_DESKTOP',
      error: expect.stringContaining('desktop'),
    });
    expect(callCoreRpc).not.toHaveBeenCalled();
  });

  it('saves to the user-chosen path and returns it', async () => {
    resolveOk();
    hoisted.invoke.mockResolvedValueOnce('/Users/me/Desktop/Deck.pptx');
    const outcome = await saveArtifactViaDialog('a-1', 'Deck', 'pptx');
    expect(outcome).toEqual({ ok: true, path: '/Users/me/Desktop/Deck.pptx' });
    expect(hoisted.invoke).toHaveBeenCalledWith('save_artifact_via_dialog', {
      sourcePath: '/ws/artifacts/a-1/deck.pptx',
      suggestedFilename: 'Deck.pptx',
    });
  });

  it('treats a null result (dialog dismissed) as CANCELLED, not an error', async () => {
    resolveOk();
    hoisted.invoke.mockResolvedValueOnce(null);
    const outcome = await saveArtifactViaDialog('a-1', 'Deck', 'pptx');
    expect(outcome).toEqual({ ok: false, code: 'CANCELLED', error: expect.any(String) });
  });

  it('falls back to the Downloads copy when the dialog is unavailable', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    // First resolve (dialog path) → invoke throws; fallback re-resolves
    // then invokes the Downloads command successfully.
    resolveOk();
    hoisted.invoke.mockRejectedValueOnce(new Error('no portal'));
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      absolute_path: '/ws/artifacts/a-1/deck.pptx',
      meta: { id: 'a-1', title: 'Deck' },
    } as never);
    hoisted.invoke.mockResolvedValueOnce('/Users/me/Downloads/Deck.pptx');

    const outcome = await saveArtifactViaDialog('a-1', 'Deck', 'pptx');
    expect(outcome).toEqual({ ok: true, path: '/Users/me/Downloads/Deck.pptx' });
    expect(hoisted.invoke).toHaveBeenNthCalledWith(2, 'download_artifact_to_downloads', {
      sourcePath: '/ws/artifacts/a-1/deck.pptx',
      filename: 'Deck.pptx',
    });
    warn.mockRestore();
  });

  it('propagates resolve failures without showing a dialog', async () => {
    vi.mocked(callCoreRpc).mockRejectedValueOnce(new Error('rpc down'));
    const outcome = await saveArtifactViaDialog('a-1', 'Deck', 'pptx');
    expect(outcome).toEqual({ ok: false, code: 'RESOLVE_FAILED', error: 'rpc down' });
    expect(hoisted.invoke).not.toHaveBeenCalled();
  });
});
