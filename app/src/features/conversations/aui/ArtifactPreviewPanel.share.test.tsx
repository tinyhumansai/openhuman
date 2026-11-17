import { act, fireEvent, render, renderHook, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../../../services/coreRpcClient';
import { ShareStatus, useArtifactShare } from './ArtifactPreviewPanel';

vi.mock('../../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('../../../lib/i18n/I18nContext', () => ({
  useT: () => ({ t: (k: string) => k, locale: 'en' }),
}));

const rpc = vi.mocked(callCoreRpc);
const writeText = vi.fn().mockResolvedValue(undefined);

describe('useArtifactShare', () => {
  beforeEach(() => {
    rpc.mockReset();
    writeText.mockClear();
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
  });

  it('saves a pending edit, uploads, then copies and shows the link', async () => {
    const order: string[] = [];
    const flush = vi.fn(async () => {
      order.push('flush');
    });
    rpc.mockImplementation(async () => {
      order.push('share');
      return {
        url: 'https://s3.example/presigned?sig=abc',
        version: 3,
        public: false,
        expires_at_millis: 1_894_060_800_000,
        reused: false,
      };
    });
    const { result } = renderHook(() => useArtifactShare('a-1', flush));

    await act(async () => result.current.share());

    expect(order).toEqual(['flush', 'share']);
    expect(rpc).toHaveBeenCalledWith({
      method: 'openhuman.ai_share_artifact',
      params: { artifact_id: 'a-1' },
    });
    expect(writeText).toHaveBeenCalledWith('https://s3.example/presigned?sig=abc');
    expect(result.current.state).toMatchObject({
      status: 'shared',
      url: 'https://s3.example/presigned?sig=abc',
      version: 3,
      copied: true,
    });
  });

  it('shows why sharing failed (e.g. signed out)', async () => {
    rpc.mockRejectedValue(new Error('sign in to share artifacts'));
    const { result } = renderHook(() => useArtifactShare('a-1'));
    await act(async () => result.current.share());
    expect(result.current.state).toEqual({
      status: 'error',
      message: 'sign in to share artifacts',
    });
    expect(writeText).not.toHaveBeenCalled();
  });

  it('still shows the link when the clipboard is refused', async () => {
    writeText.mockRejectedValueOnce(new Error('denied'));
    rpc.mockResolvedValue({ url: 'https://x', version: 1, public: true, reused: true });
    const { result } = renderHook(() => useArtifactShare('a-1'));
    await act(async () => result.current.share());
    expect(result.current.state).toMatchObject({
      status: 'shared',
      copied: false,
      expiresAt: null,
    });

    await act(async () => result.current.copyAgain());
    expect(result.current.state).toMatchObject({ copied: true });
  });
});

describe('ShareStatus copy button', () => {
  beforeEach(() => {
    rpc.mockReset();
    writeText.mockClear();
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
  });

  it('copies the full link, query string included', async () => {
    const url =
      'https://s3.example/f?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=c&X-Amz-Signature=s';
    rpc.mockResolvedValue({ url, version: 2, public: false, reused: false });
    function Harness() {
      const sharing = useArtifactShare('a-1');
      return (
        <>
          <button onClick={() => void sharing.share()}>share</button>
          <ShareStatus state={sharing.state} onCopy={sharing.copyAgain} />
        </>
      );
    }
    render(<Harness />);
    await act(async () => fireEvent.click(screen.getByText('share')));
    // The share copied the link already; the button still offers to copy.
    expect(screen.getByTestId('artifact-share-copy').textContent).toBe('chat.artifact.share.copy');
    writeText.mockClear();

    await act(async () => fireEvent.click(screen.getByTestId('artifact-share-copy')));
    expect(writeText).toHaveBeenCalledWith(url);
    expect(screen.getByTestId('artifact-share-copy').textContent).toContain('chat.artifact.copied');
  });
});
