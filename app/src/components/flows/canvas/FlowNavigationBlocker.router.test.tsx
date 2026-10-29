import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { useRef, useState } from 'react';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { RouterProvider, useLocation, useNavigate } from 'react-router-dom';

import { createAppHashRouter } from '../../../lib/navigation/createAppHashRouter';
import FlowNavigationBlocker, { type BlockedFlowNavigation } from './FlowNavigationBlocker';

function DirtyCanvasProbe() {
  const navigate = useNavigate();
  const location = useLocation();
  const bypassRef = useRef(false);
  const blockedRef = useRef<BlockedFlowNavigation | null>(null);
  const [asking, setAsking] = useState(false);

  return (
    <div>
      <FlowNavigationBlocker
        dirty
        bypassRef={bypassRef}
        blockedRef={blockedRef}
        onBlocked={() => setAsking(true)}
      />
      <output data-testid="pathname">{location.pathname}</output>
      <button type="button" onClick={() => navigate('/chat')}>
        Open chat
      </button>
      {asking && (
        <div role="dialog" aria-label="Discard unsaved changes">
          <button
            type="button"
            onClick={() => {
              blockedRef.current?.reset();
              setAsking(false);
            }}>
            Stay
          </button>
          <button
            type="button"
            onClick={() => {
              blockedRef.current?.proceed();
              setAsking(false);
            }}>
            Discard
          </button>
        </div>
      )}
    </div>
  );
}

describe('production hash-data-router flow navigation', () => {
  beforeEach(() => {
    window.history.replaceState(null, '', '#/flows/one');
  });
  afterEach(() => {
    window.history.replaceState(null, '', '#/');
  });

  it('blocks sidebar navigation until the user confirms leaving a dirty canvas', async () => {
    // Exercise the same router factory mounted by App, not a test-only memory router.
    const router = createAppHashRouter(<DirtyCanvasProbe />);
    render(<RouterProvider router={router} />);

    expect(screen.getByTestId('pathname')).toHaveTextContent('/flows/one');
    fireEvent.click(screen.getByRole('button', { name: 'Open chat' }));
    expect(await screen.findByRole('dialog', { name: 'Discard unsaved changes' })).toBeInTheDocument();
    expect(screen.getByTestId('pathname')).toHaveTextContent('/flows/one');

    fireEvent.click(screen.getByRole('button', { name: 'Stay' }));
    await waitFor(() =>
      expect(screen.queryByRole('dialog', { name: 'Discard unsaved changes' })).not.toBeInTheDocument()
    );
    expect(screen.getByTestId('pathname')).toHaveTextContent('/flows/one');

    fireEvent.click(screen.getByRole('button', { name: 'Open chat' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Discard' }));
    await waitFor(() => expect(screen.getByTestId('pathname')).toHaveTextContent('/chat'));
  });
});
