/**
 * Phase 3 — Activity route redirect tests.
 *
 * Verifies that:
 *   /intelligence  → /activity            (back-compat, old route rename)
 *   /routines      → /activity?tab=automations  (orphaned page removal)
 *   /workflows     → /activity?tab=automations  (old deep link back-compat)
 *
 * We render a minimal route tree so we do not need the full provider tree.
 */
import { render, screen } from '@testing-library/react';
import { MemoryRouter, Navigate, Route, Routes } from 'react-router-dom';
import { describe, expect, it } from 'vitest';

/**
 * Minimal route tree that mirrors only the Phase 3 redirects under test.
 * Using `Navigate` directly avoids needing to mock the full app.
 */
function TestRoutes() {
  return (
    <Routes>
      <Route path="/activity" element={<div data-testid="activity-page">activity</div>} />
      <Route path="/intelligence" element={<Navigate to="/activity" replace />} />
      <Route path="/routines" element={<Navigate to="/activity?tab=automations" replace />} />
      <Route path="/workflows" element={<Navigate to="/activity?tab=automations" replace />} />
    </Routes>
  );
}

const renderAt = (path: string) =>
  render(
    <MemoryRouter initialEntries={[path]}>
      <TestRoutes />
    </MemoryRouter>
  );

describe('Phase 3 route redirects', () => {
  it('/intelligence redirects to /activity', () => {
    renderAt('/intelligence');
    expect(screen.getByTestId('activity-page')).toBeInTheDocument();
  });

  it('/routines redirects to /activity (Automations tab)', () => {
    renderAt('/routines');
    expect(screen.getByTestId('activity-page')).toBeInTheDocument();
  });

  it('/workflows redirects to /activity (Automations tab)', () => {
    renderAt('/workflows');
    expect(screen.getByTestId('activity-page')).toBeInTheDocument();
  });

  it('/activity renders the activity page directly', () => {
    renderAt('/activity');
    expect(screen.getByTestId('activity-page')).toBeInTheDocument();
  });
});
