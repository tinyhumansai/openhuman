import { isTauri as runtimeIsTauri } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { getCurrent, onOpenUrl } from '@tauri-apps/plugin-deep-link';
import { beforeEach, describe, expect, it, type Mock, vi } from 'vitest';

import { consumeLoginToken } from '../../services/api/authApi';
import { store } from '../../store';
import { clearToken } from '../../store/authSlice';
import { setupDesktopDeepLinkListener } from '../desktopDeepLinkListener';

vi.mock('../../services/api/authApi', () => ({ consumeLoginToken: vi.fn() }));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: vi.fn() }));

describe('desktopDeepLinkListener', () => {
  const mockIsTauri = runtimeIsTauri as Mock;
  const mockGetCurrent = getCurrent as Mock;
  const mockOnOpenUrl = onOpenUrl as Mock;
  const mockConsumeLoginToken = consumeLoginToken as Mock;
  const mockGetCurrentWindow = getCurrentWindow as Mock;

  beforeEach(async () => {
    vi.clearAllMocks();
    await store.dispatch(clearToken());
    window.location.hash = '/';

    mockIsTauri.mockReturnValue(true);
    mockGetCurrent.mockResolvedValue(null);
    mockOnOpenUrl.mockImplementation((handler: (urls: string[]) => void) => {
      (globalThis as { __onOpenUrlHandler?: (urls: string[]) => void }).__onOpenUrlHandler =
        handler;
      return Promise.resolve(() => {});
    });
    mockGetCurrentWindow.mockReturnValue({
      show: vi.fn().mockResolvedValue(undefined),
      unminimize: vi.fn().mockResolvedValue(undefined),
      setFocus: vi.fn().mockResolvedValue(undefined),
    });
  });

  it('applies bypass auth token from deep link and routes to home', async () => {
    await setupDesktopDeepLinkListener();
    const handler = (globalThis as { __onOpenUrlHandler?: (urls: string[]) => void })
      .__onOpenUrlHandler;
    expect(handler).toBeTypeOf('function');

    handler?.(['openhuman://auth?token=test-bypass-token&key=auth']);

    await vi.waitFor(() => expect(store.getState().auth.token).toBe('test-bypass-token'), {
      timeout: 4000,
    });
    expect(window.location.hash).toBe('#/home');
    expect(mockConsumeLoginToken).not.toHaveBeenCalled();
  });

  it('consumes login token through API for non-bypass auth deep links', async () => {
    mockConsumeLoginToken.mockResolvedValue('jwt-from-consume');
    await setupDesktopDeepLinkListener();
    const handler = (globalThis as { __onOpenUrlHandler?: (urls: string[]) => void })
      .__onOpenUrlHandler;

    handler?.(['openhuman://auth?token=oauth-token']);

    await vi.waitFor(() => expect(mockConsumeLoginToken).toHaveBeenCalledWith('oauth-token'), {
      timeout: 4000,
    });
    await vi.waitFor(() => expect(store.getState().auth.token).toBe('jwt-from-consume'), {
      timeout: 4000,
    });
    expect(window.location.hash).toBe('#/home');
  });

  it('ignores unsupported deep-link schemes', async () => {
    await setupDesktopDeepLinkListener();
    const handler = (globalThis as { __onOpenUrlHandler?: (urls: string[]) => void })
      .__onOpenUrlHandler;

    handler?.(['https://example.com/auth?token=not-openhuman']);

    await new Promise(resolve => setTimeout(resolve, 20));
    expect(store.getState().auth.token).toBeNull();
    expect(mockConsumeLoginToken).not.toHaveBeenCalled();
  });
});
