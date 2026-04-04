import { render, screen } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';

import PublicRoute from '../PublicRoute';

const mockUseCoreState = vi.fn();

vi.mock('../../providers/CoreStateProvider', () => ({ useCoreState: () => mockUseCoreState() }));

function renderRoute(routes: React.ReactNode, initialEntries = ['/']) {
  return render(
    <MemoryRouter initialEntries={initialEntries}>
      <Routes>{routes}</Routes>
    </MemoryRouter>
  );
}

describe('PublicRoute', () => {
  it('renders children when user is not authenticated', () => {
    mockUseCoreState.mockReturnValue({ isBootstrapping: false, snapshot: { sessionToken: null } });

    renderRoute(
      <Route
        path="/"
        element={
          <PublicRoute>
            <div>Welcome Page</div>
          </PublicRoute>
        }
      />
    );

    expect(screen.getByText('Welcome Page')).toBeInTheDocument();
  });

  it('redirects to /home when user is authenticated', () => {
    mockUseCoreState.mockReturnValue({
      isBootstrapping: false,
      snapshot: { sessionToken: 'jwt-token' },
    });

    renderRoute(
      <>
        <Route
          path="/"
          element={
            <PublicRoute>
              <div>Welcome Page</div>
            </PublicRoute>
          }
        />
        <Route path="/home" element={<div>Home</div>} />
      </>
    );

    expect(screen.queryByText('Welcome Page')).not.toBeInTheDocument();
    expect(screen.getByText('Home')).toBeInTheDocument();
  });

  it('redirects to custom path when authenticated', () => {
    mockUseCoreState.mockReturnValue({
      isBootstrapping: false,
      snapshot: { sessionToken: 'jwt-token' },
    });

    renderRoute(
      <>
        <Route
          path="/"
          element={
            <PublicRoute redirectTo="/dashboard">
              <div>Welcome Page</div>
            </PublicRoute>
          }
        />
        <Route path="/dashboard" element={<div>Dashboard</div>} />
      </>
    );

    expect(screen.getByText('Dashboard')).toBeInTheDocument();
  });
});
