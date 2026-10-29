import { fireEvent, render, screen } from '@testing-library/react';
import { RouterProvider } from 'react-router-dom';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { createAppHashRouter } from './createAppHashRouter';

vi.mock('../../components/ErrorFallbackScreen', () => ({
  default: ({ error, onReset }: { error: unknown; onReset: () => void }) => (
    <button type="button" onClick={onReset}>
      Recover from {error instanceof Error ? error.message : String(error)}
    </button>
  ),
}));

afterEach(() => vi.restoreAllMocks());

describe('createAppHashRouter', () => {
  it('retains Sentry crash recovery inside the data router after a route render failure', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    const previousHash = window.location.hash;
    window.location.hash = '#/test-crash';
    let shouldCrash = true;
    function Shell() {
      if (shouldCrash) throw new Error('route-render-canary');
      return <div>Recovered shell</div>;
    }
    const router = createAppHashRouter(<Shell />);
    try {
      const view = render(<RouterProvider router={router} />);
      const recovery = await screen.findByRole('button', {
        name: /Recover from route-render-canary/,
      });
      shouldCrash = false;
      fireEvent.click(recovery);
      expect(await screen.findByText('Recovered shell')).toBeInTheDocument();
      view.unmount();
    } finally {
      window.location.hash = previousHash;
      router.dispose();
    }
  });
});
