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
