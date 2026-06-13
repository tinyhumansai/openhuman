import { render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { handleDeepLinkUrls } from '../../utils/desktopDeepLinkListener';
import WebCallbackPage from '../WebCallbackPage';

vi.mock('../../utils/desktopDeepLinkListener', () => ({ handleDeepLinkUrls: vi.fn() }));

describe('WebCallbackPage', () => {
  afterEach(() => {
    vi.clearAllMocks();
  });

  function renderRoute(initialEntry: string) {
    return render(
      <MemoryRouter initialEntries={[initialEntry]}>
        <Routes>
          <Route path="/callback/:kind" element={<WebCallbackPage />} />
          <Route path="/callback/:kind/:status" element={<WebCallbackPage />} />
        </Routes>
      </MemoryRouter>
    );
  }

  it('routes auth callbacks through the synthetic auth deep link handler', async () => {
    renderRoute('/callback/auth?token=jwt-token&key=auth');

    expect(screen.getByText('Completing sign-in')).toBeInTheDocument();
    await waitFor(() => {
      expect(handleDeepLinkUrls).toHaveBeenCalledWith([
        'openhuman://auth?token=jwt-token&key=auth',
      ]);
    });
  });

  it('routes oauth callbacks through the synthetic oauth deep link handler', async () => {
    renderRoute('/callback/oauth/success?provider=google&integrationId=int-1');

    await waitFor(() => {
      expect(handleDeepLinkUrls).toHaveBeenCalledWith([
        'openhuman://oauth/success?provider=google&integrationId=int-1',
      ]);
    });
  });

  it('does not emit a synthetic deep link for unsupported callback shapes', async () => {
    renderRoute('/callback/oauth');

    expect(screen.getByText(/processing your callback/i)).toBeInTheDocument();
    await waitFor(() => {
      expect(handleDeepLinkUrls).not.toHaveBeenCalled();
    });
  });
});
