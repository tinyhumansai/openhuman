/**
 * Desktop `/human` and `/chat` routes.
 *
 * Both surfaces carry the mascot on purpose — `/human` is the dedicated stage,
 * `/chat` docks it on the composer — so this pins that each route serves its own
 * page and neither silently redirects to the other.
 *
 * Renders the REAL `AppRoutes` with a location probe. An earlier version of this
 * file declared its own local route tree, which meant it asserted a fixture
 * against itself and kept passing while the routing changed underneath it.
 */
import { render, screen } from '@testing-library/react';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';

// Stub the routed surfaces so this test needs no provider chain — the subject
// here is the route table, not the pages.
vi.mock('../Accounts', () => ({ default: () => <div data-testid="chat-page">chat</div> }));
vi.mock('../../features/human/HumanPage', () => ({
  default: () => <div data-testid="human-page">human</div>,
}));
vi.mock('../../components/ProtectedRoute', () => ({
  default: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
vi.mock('../../components/PublicRoute', () => ({
  default: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
vi.mock('../../components/DefaultRedirect', () => ({ default: () => <div /> }));

const AppRoutes = (await import('../../AppRoutes')).default;

/** Renders the resolved pathname, so a silent redirect cannot pass unnoticed. */
const LocationProbe = () => <span data-testid="pathname">{useLocation().pathname}</span>;

const renderAt = (path: string) =>
  render(
    <MemoryRouter initialEntries={[path]}>
      <AppRoutes />
      <LocationProbe />
    </MemoryRouter>
  );

describe('Desktop /human + /chat routes', () => {
  it('serves the Human page at /human without redirecting', () => {
    renderAt('/human');
    expect(screen.getByTestId('pathname')).toHaveTextContent('/human');
    expect(screen.getByTestId('human-page')).toBeInTheDocument();
    expect(screen.queryByTestId('chat-page')).not.toBeInTheDocument();
  });

  it('serves the chat surface at /chat', () => {
    renderAt('/chat');
    expect(screen.getByTestId('pathname')).toHaveTextContent('/chat');
    expect(screen.getByTestId('chat-page')).toBeInTheDocument();
    expect(screen.queryByTestId('human-page')).not.toBeInTheDocument();
  });
});
