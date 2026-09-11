import { render, screen, waitFor } from '@testing-library/react';
import { HashRouter, Outlet, Route, Routes } from 'react-router-dom';
import { describe, expect, it } from 'vitest';

import { settingsRouteElements } from './settingsRouteElements';

describe.each([
  '/settings/features',
  '/settings/screen-intelligence',
  '/settings/screen-awareness-debug',
])('retired screen settings route %s', route => {
  it('normalizes to the settings index without rendering removed content', async () => {
    window.location.hash = `#${route}`;

    render(
      <HashRouter>
        <Routes>
          <Route path="/settings" element={<Outlet />}>
            {settingsRouteElements()}
          </Route>
        </Routes>
      </HashRouter>
    );

    await waitFor(() => expect(window.location.hash).toBe('#/settings'));
    expect(screen.queryByText(/screen intelligence/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/screen awareness debug/i)).not.toBeInTheDocument();
  });
});

/**
 * Hop two of the `/webhooks` chain (#5908).
 *
 *   /webhooks              -> /settings/integrations   AppRoutes.tsx
 *   /settings/integrations -> /connections             here
 *
 * Both hops used a bare `<Navigate>`, which discards `search` and `hash`.
 * Fixing only the first was not enough — the deep link reached
 * `/settings/integrations` and was dropped here. This route is also reachable on
 * its own by anyone holding an old Integrations settings link, so it needs the
 * forwarding regardless of how they arrived.
 *
 * Rendered through the real route table rather than through `AppRoutes`, because
 * every AppRoutes spec stubs `./pages/Settings` and the stub removes this hop.
 */
describe('retired /settings/integrations route', () => {
  function renderAt(hash: string) {
    window.location.hash = `#${hash}`;
    render(
      <HashRouter>
        <Routes>
          <Route path="/settings" element={<Outlet />}>
            {settingsRouteElements()}
          </Route>
        </Routes>
      </HashRouter>
    );
  }

  it('redirects to /connections', async () => {
    renderAt('/settings/integrations');
    await waitFor(() => expect(window.location.hash).toBe('#/connections'));
  });

  it('carries a query string and a fragment through the redirect', async () => {
    renderAt('/settings/integrations?tab=inbound#delivery-3');
    await waitFor(() => expect(window.location.hash).toBe('#/connections?tab=inbound#delivery-3'));
  });

  it('invents no stray ? or # when the link carries neither', async () => {
    renderAt('/settings/integrations');
    await waitFor(() => expect(window.location.hash).toBe('#/connections'));
  });
});
