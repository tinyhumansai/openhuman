import { screen } from '@testing-library/react';
import { Route, Routes } from 'react-router-dom';
import { describe, expect, it } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import ProtectedRoute from '../ProtectedRoute';

describe('ProtectedRoute', () => {
  it('renders children when token exists', () => {
    renderWithProviders(
      <Routes>
        <Route
          path="/"
          element={
            <ProtectedRoute>
              <div>Protected Content</div>
            </ProtectedRoute>
          }
        />
      </Routes>,
      {
        preloadedState: {
          auth: { token: 'valid-jwt', isOnboardedByUser: {}, isAnalyticsEnabledByUser: {} },
        },
      }
    );

    expect(screen.getByText('Protected Content')).toBeInTheDocument();
  });

  it('redirects to / when no token and requireAuth=true', () => {
    renderWithProviders(
      <Routes>
        <Route
          path="/dashboard"
          element={
            <ProtectedRoute>
              <div>Dashboard</div>
            </ProtectedRoute>
          }
        />
        <Route path="/" element={<div>Landing</div>} />
      </Routes>,
      {
        initialEntries: ['/dashboard'],
        preloadedState: {
          auth: { token: null, isOnboardedByUser: {}, isAnalyticsEnabledByUser: {} },
        },
      }
    );

    expect(screen.queryByText('Dashboard')).not.toBeInTheDocument();
    expect(screen.getByText('Landing')).toBeInTheDocument();
  });

  it('redirects to custom redirectTo when no token', () => {
    renderWithProviders(
      <Routes>
        <Route
          path="/dashboard"
          element={
            <ProtectedRoute redirectTo="/login">
              <div>Dashboard</div>
            </ProtectedRoute>
          }
        />
        <Route path="/login" element={<div>Login Page</div>} />
      </Routes>,
      {
        initialEntries: ['/dashboard'],
        preloadedState: {
          auth: { token: null, isOnboardedByUser: {}, isAnalyticsEnabledByUser: {} },
        },
      }
    );

    expect(screen.getByText('Login Page')).toBeInTheDocument();
  });

  it('redirects to /onboarding when requireOnboarded but not onboarded', () => {
    renderWithProviders(
      <Routes>
        <Route
          path="/home"
          element={
            <ProtectedRoute requireOnboarded>
              <div>Home Content</div>
            </ProtectedRoute>
          }
        />
        <Route path="/onboarding" element={<div>Onboarding</div>} />
      </Routes>,
      {
        initialEntries: ['/home'],
        preloadedState: {
          auth: { token: 'valid-jwt', isOnboardedByUser: {}, isAnalyticsEnabledByUser: {} },
          user: { user: { _id: 'u1' }, isLoading: false, error: null },
        },
      }
    );

    expect(screen.getByText('Onboarding')).toBeInTheDocument();
  });
});
