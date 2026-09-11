/**
 * Back-compat redirect contracts for the desktop route table.
 *
 * Every `<Navigate>` in `AppRoutes.tsx` is a promise to somebody's bookmark: a
 * route that used to be top-level and now lives somewhere else. Breaking one is
 * silent — the app still renders *a* page, just not the one the link meant.
 *
 * This asserts a different contract from `test/playwright/specs/navigation.spec.ts`,
 * which covers four of these at the browser layer. That spec asserts the hash
 * *begins with* the landing path, which cannot tell `/connections` apart from
 * `/connections?tab=messaging`, and says nothing about `replace`. Here the whole
 * resolved location is compared exactly, and the `replace` semantics — the thing
 * that stops the Back button bouncing a user into a redirect loop — are asserted
 * on their own.
 *
 * Mocks mirror `AppRoutes.auth.test.tsx`: every leaf page is stubbed so this
 * exercises the router and nothing else.
 */
import { render, screen } from '@testing-library/react';
import type React from 'react';
import { MemoryRouter, useLocation, useNavigationType } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';

vi.mock('./lib/platform', () => ({ getIsMobile: () => false }));

vi.mock('./components/PublicRoute', () => ({
  default: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
vi.mock('./components/ProtectedRoute', () => ({
  default: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
vi.mock('./components/DefaultRedirect', () => ({
  default: () => <div data-testid="default-redirect" />,
}));

vi.mock('./pages/WebCallbackPage', () => ({ default: () => <div /> }));
vi.mock('./AppRoutesIOS', () => ({ default: () => <div /> }));
vi.mock('./features/human/HumanPage', () => ({ default: () => <div /> }));
vi.mock('./pages/Accounts', () => ({ default: () => <div /> }));
vi.mock('./pages/Brain', () => ({ default: () => <div /> }));
vi.mock('./pages/dev/AgentInsightsPreview', () => ({ default: () => <div /> }));
vi.mock('./pages/dev/UiGallery', () => ({ default: () => <div /> }));
vi.mock('./pages/Invites', () => ({ default: () => <div /> }));
vi.mock('./pages/Notifications', () => ({ default: () => <div /> }));
vi.mock('./pages/onboarding/Onboarding', () => ({ default: () => <div /> }));
vi.mock('./pages/PttOverlayPage', () => ({ PttOverlayPage: () => <div /> }));
vi.mock('./pages/Rewards', () => ({ default: () => <div /> }));
vi.mock('./pages/Settings', () => ({ default: () => <div /> }));
vi.mock('./pages/Skills', () => ({ default: () => <div /> }));
vi.mock('./pages/Welcome', () => ({ default: () => <div /> }));
vi.mock('./pages/WorkflowsRun', () => ({ default: () => <div /> }));

const AppRoutes = (await import('./AppRoutes')).default;

/** Reports the router's resolved location, so a redirect is observed rather than inferred. */
function LocationProbe() {
  const location = useLocation();
  return (
    <>
      <div data-testid="pathname">{location.pathname}</div>
      <div data-testid="search">{location.search}</div>
    </>
  );
}

/** Reports the router's last navigation action, so `replace` is observed rather than assumed. */
function ActionProbe() {
  return <div data-testid="nav-type">{useNavigationType()}</div>;
}

function landingFor(entry: string) {
  render(
    <MemoryRouter initialEntries={[entry]}>
      <AppRoutes />
      <LocationProbe />
    </MemoryRouter>
  );
  return {
    pathname: screen.getByTestId('pathname').textContent,
    search: screen.getByTestId('search').textContent,
  };
}

// Every `<Navigate>` in AppRoutes.tsx, read off the route table. If a redirect
// is added there and not here, `every redirect in the route table is covered`
// below fails — the list cannot silently fall behind the code.
const REDIRECTS: Array<{ from: string; pathname: string; search: string }> = [
  { from: '/home', pathname: '/chat', search: '' },
  { from: '/activity', pathname: '/settings/notifications', search: '' },
  { from: '/intelligence', pathname: '/settings/notifications', search: '' },
  { from: '/skills', pathname: '/connections', search: '' },
  { from: '/accounts', pathname: '/chat', search: '' },
  { from: '/channels', pathname: '/connections', search: '?tab=messaging' },
  { from: '/feedback', pathname: '/settings/feedback', search: '' },
  { from: '/routines', pathname: '/flows', search: '' },
  { from: '/webhooks', pathname: '/settings/integrations', search: '' },
];

describe('AppRoutes back-compat redirects', () => {
  for (const { from, pathname, search } of REDIRECTS) {
    it(`sends ${from} to ${pathname}${search}`, () => {
      expect(landingFor(from)).toEqual({ pathname, search });
    });
  }

  it('carries the messaging tab through the /channels redirect', () => {
    // Singled out because it is the only redirect whose contract is a query
    // string. A prefix match on the path alone passes even when `?tab=messaging`
    // is dropped, which lands the user on the wrong Connections tab.
    expect(landingFor('/channels').search).toBe('?tab=messaging');
  });

  it('every redirect in the route table is covered by this spec', async () => {
    const [fs, path] = await Promise.all([import('node:fs'), import('node:path')]);
    // vitest runs with `app/` as cwd (test/vitest.config.ts sets the root).
    const source = fs.readFileSync(path.resolve('src/AppRoutes.tsx'), 'utf8');

    // Each route's body is the slice up to the next `<Route path=`, and the body
    // is searched for `<Navigate`. Deliberately NOT a single-line regex like
    // `/<Route path="…" element={<Navigate /`: Prettier wraps a `<Route>` across
    // lines as soon as its props exceed the print width, and a one-line pattern
    // then silently skips that redirect — the list stays unchanged and this test
    // still claims full coverage. A guard that stops guarding is worse than none.
    const starts = [...source.matchAll(/<Route\s+path="([^"]+)"/g)];
    const routed = starts
      .filter((match, index) => {
        const from = match.index ?? 0;
        const to =
          index + 1 < starts.length ? (starts[index + 1].index ?? source.length) : source.length;
        // `ForwardSearch` (#5924) renders `<Navigate>` internally after copying
        // the query string and hash, so the route body has no literal
        // `<Navigate`. Matching only that dropped `/skills` from this list.
        return /<(?:Navigate|ForwardSearch)\b/.test(source.slice(from, to));
      })
      .map(match => match[1]);

    expect(routed.sort()).toEqual(REDIRECTS.map(r => r.from).sort());
  });
});

describe('AppRoutes redirects replace rather than push', () => {
  // A redirect that pushes leaves the retired path on the history stack, so
  // Back returns to it and is immediately redirected forward again — the user
  // cannot leave. `replace` is what prevents that. Nothing else asserts it, and
  // dropping `replace` from a `<Navigate>` is a one-word edit that every other
  // test in the tree still passes.
  for (const { from } of REDIRECTS) {
    it(`${from} navigates by REPLACE, not PUSH`, () => {
      render(
        <MemoryRouter initialEntries={[from]}>
          <AppRoutes />
          <ActionProbe />
        </MemoryRouter>
      );
      expect(screen.getByTestId('nav-type')).toHaveTextContent('REPLACE');
    });
  }
});
