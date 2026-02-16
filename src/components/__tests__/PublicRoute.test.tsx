import { screen } from '@testing-library/react';
import { Route, Routes } from 'react-router-dom';
import { describe, expect, it } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import PublicRoute from '../PublicRoute';

describe('PublicRoute', () => {
  it('renders children when user is not authenticated', () => {
    renderWithProviders(
      <Routes>
        <Route
          path="/"
          element={
            <PublicRoute>
              <div>Welcome Page</div>
            </PublicRoute>
          }
        />
      </Routes>,
      {
        preloadedState: {
          auth: { token: null, isOnboardedByUser: {}, isAnalyticsEnabledByUser: {} },
        },
      }
    );

    expect(screen.getByText('Welcome Page')).toBeInTheDocument();
  });

  it('redirects to /home when user is authenticated', () => {
    renderWithProviders(
      <Routes>
        <Route
          path="/"
          element={
            <PublicRoute>
              <div>Welcome Page</div>
            </PublicRoute>
          }
        />
        <Route path="/home" element={<div>Home</div>} />
      </Routes>,
      {
        preloadedState: {
          auth: { token: 'jwt-token', isOnboardedByUser: {}, isAnalyticsEnabledByUser: {} },
        },
      }
    );

    expect(screen.queryByText('Welcome Page')).not.toBeInTheDocument();
    expect(screen.getByText('Home')).toBeInTheDocument();
  });

  it('redirects to custom path when authenticated', () => {
    renderWithProviders(
      <Routes>
        <Route
          path="/"
          element={
            <PublicRoute redirectTo="/dashboard">
              <div>Welcome Page</div>
            </PublicRoute>
          }
        />
        <Route path="/dashboard" element={<div>Dashboard</div>} />
      </Routes>,
      {
        preloadedState: {
          auth: { token: 'jwt-token', isOnboardedByUser: {}, isAnalyticsEnabledByUser: {} },
        },
      }
    );

    expect(screen.getByText('Dashboard')).toBeInTheDocument();
  });
});
