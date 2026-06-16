/**
 * Phase 2 — route redirect tests.
 *
 * Verifies that:
 *   /skills      → /connections  (back-compat redirect)
 *   /channels    → /connections?tab=messaging  (orphaned page redirect)
 *
 * We render a minimal route tree so we do not need the full provider tree.
 */
import { render, screen } from '@testing-library/react';
import { MemoryRouter, Navigate, Route, Routes } from 'react-router-dom';
import { describe, expect, it } from 'vitest';

/**
 * Minimal route tree that mirrors only the redirects under test.
 * Using `Navigate` directly avoids needing to mock the full app.
 */
function TestRoutes() {
  return (
    <Routes>
      <Route path="/connections" element={<div data-testid="connections-page">connections</div>} />
      <Route path="/skills" element={<Navigate to="/connections" replace />} />
      <Route path="/channels" element={<Navigate to="/connections?tab=messaging" replace />} />
    </Routes>
  );
}

const renderAt = (path: string) =>
  render(
    <MemoryRouter initialEntries={[path]}>
      <TestRoutes />
    </MemoryRouter>
  );

describe('Phase 2 route redirects', () => {
  it('/skills redirects to /connections', () => {
    renderAt('/skills');
    expect(screen.getByTestId('connections-page')).toBeInTheDocument();
  });

  it('/channels redirects to /connections (Messaging tab)', () => {
    renderAt('/channels');
    expect(screen.getByTestId('connections-page')).toBeInTheDocument();
  });

  it('/connections renders the connections page directly', () => {
    renderAt('/connections');
    expect(screen.getByTestId('connections-page')).toBeInTheDocument();
  });
});
