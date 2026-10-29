import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { useRef, useState } from 'react';
import {
  createHashRouter,
  createMemoryRouter,
  RouterProvider,
  useNavigate,
} from 'react-router-dom';
import { describe, expect, it } from 'vitest';

import FlowNavigationBlocker, { type BlockedFlowNavigation } from './FlowNavigationBlocker';

function GuardedEditor() {
  const navigate = useNavigate();
  const [dirty, setDirty] = useState(true);
  const [confirm, setConfirm] = useState(false);
  const bypassRef = useRef(false);
  const blockedRef = useRef<BlockedFlowNavigation | null>(null);
  return (
    <div>
      <FlowNavigationBlocker
        dirty={dirty}
        bypassRef={bypassRef}
        blockedRef={blockedRef}
        onBlocked={() => setConfirm(true)}
      />
      <button type="button" onClick={() => navigate('/chat')}>
        Open chat
      </button>
      <button type="button" onClick={() => navigate(-1)}>
        Browser back
      </button>
      <button
        type="button"
        onClick={() => {
          bypassRef.current = true;
          navigate('/chat');
        }}>
        Save and continue
      </button>
      <button
        type="button"
        onClick={() => {
          bypassRef.current = true;
          navigate('/flows/test-id?panel=history', { replace: true });
        }}>
        Save current route
      </button>
      <button type="button" onClick={() => setDirty(false)}>
        Discard edits
      </button>
      {confirm && (
        <div role="dialog" aria-label="Unsaved workflow">
          <button
            type="button"
            onClick={() => {
              blockedRef.current?.reset();
              setConfirm(false);
            }}>
            Stay
          </button>
          <button
            type="button"
            onClick={() => {
              blockedRef.current?.proceed();
              setConfirm(false);
            }}>
            Discard and leave
          </button>
        </div>
      )}
    </div>
  );
}

function setup(initialEntries = ['/flows/test-id']) {
  const router = createMemoryRouter(
    [
      { path: '/flows/:id', element: <GuardedEditor /> },
      { path: '/chat', element: <p data-testid="chat-page">Chat</p> },
      { path: '/home', element: <p data-testid="home-page">Home</p> },
    ],
    { initialEntries }
  );
  render(<RouterProvider router={router} />);
  return router;
}

describe('FlowNavigationBlocker', () => {
  it('guards dirty editor navigation under the production hash data router', async () => {
    const originalUrl = window.location.href;
    window.history.replaceState(null, '', '/#/flows/test-id');
    const router = createHashRouter([
      { path: '/flows/:id', element: <GuardedEditor /> },
      { path: '/chat', element: <p data-testid="chat-page">Chat</p> },
    ]);
    const view = render(<RouterProvider router={router} />);
    try {
      fireEvent.click(screen.getByRole('button', { name: 'Open chat' }));
      await screen.findByRole('dialog', { name: 'Unsaved workflow' });
      expect(window.location.hash).toBe('#/flows/test-id');

      fireEvent.click(screen.getByRole('button', { name: 'Stay' }));
      expect(window.location.hash).toBe('#/flows/test-id');
      fireEvent.click(screen.getByRole('button', { name: 'Open chat' }));
      fireEvent.click(await screen.findByRole('button', { name: 'Discard and leave' }));
      await screen.findByTestId('chat-page');
      expect(window.location.hash).toBe('#/chat');
    } finally {
      view.unmount();
      router.dispose();
      window.history.replaceState(null, '', originalUrl);
    }
  });

  it('stops sidebar-like navigation until the user discards or stays', async () => {
    const router = setup();
    fireEvent.click(screen.getByRole('button', { name: 'Open chat' }));
    await screen.findByRole('dialog', { name: 'Unsaved workflow' });
    expect(router.state.location.pathname).toBe('/flows/test-id');

    fireEvent.click(screen.getByRole('button', { name: 'Stay' }));
    expect(router.state.location.pathname).toBe('/flows/test-id');

    fireEvent.click(screen.getByRole('button', { name: 'Open chat' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Discard and leave' }));
    await waitFor(() => expect(router.state.location.pathname).toBe('/chat'));
  });

  it('blocks browser-history navigation to another route', async () => {
    const router = setup(['/home', '/flows/test-id']);
    fireEvent.click(screen.getByRole('button', { name: 'Browser back' }));
    await screen.findByRole('dialog', { name: 'Unsaved workflow' });
    expect(router.state.location.pathname).toBe('/flows/test-id');
    fireEvent.click(screen.getByRole('button', { name: 'Discard and leave' }));
    await waitFor(() => expect(router.state.location.pathname).toBe('/home'));
  });

  it('consumes a same-route bypass before a later unsaved navigation', async () => {
    const router = setup();
    fireEvent.click(screen.getByRole('button', { name: 'Save current route' }));
    await waitFor(() => expect(router.state.location.search).toBe('?panel=history'));
    fireEvent.click(screen.getByRole('button', { name: 'Open chat' }));
    await screen.findByRole('dialog', { name: 'Unsaved workflow' });
    expect(router.state.location.pathname).toBe('/flows/test-id');
  });

  it('permits deliberate Save-and-navigate transitions without a discard prompt', async () => {
    const router = setup();
    fireEvent.click(screen.getByRole('button', { name: 'Save and continue' }));
    await waitFor(() => expect(router.state.location.pathname).toBe('/chat'));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
});
