/**
 * Which desktop routes require authentication, and which deliberately do not.
 *
 * A route silently losing its `<ProtectedRoute>` wrapper exposes a surface — and
 * the RPC calls it makes on mount — to an unauthenticated session. A route
 * gaining one locks out a flow that has to work signed-out: the OAuth callback
 * that *establishes* the session, and the push-to-talk overlay window.
 * Both directions are one-line edits in a 260-line JSX table, and nothing
 * asserted either.
 *
 * `test/playwright/specs/auth-access-control.spec.ts` covers the session
 * *lifecycle* (sign-in, logout, expiry) with a booted authenticated page; it
 * never varies the guard on a route. This is the complementary half: the
 * classification itself.
 *
 * Two layers, deliberately:
 *  1. Render assertions for the routes this file owns — the page renders AND
 *     the expected guard wrapped it.
 *  2. A source-level classification guard over the WHOLE route table, so a
 *     route added or re-guarded anywhere in `AppRoutes.tsx` fails here even
 *     though its page is another spec's business.
 */
import { render, screen } from '@testing-library/react';
import type React from 'react';
import { MemoryRouter, useParams } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';

vi.mock('./lib/platform', () => ({ getIsMobile: () => false }));

// The guards are replaced by markers that record that they wrapped something,
// rather than being stubbed away as passthroughs. That is the whole point here.
vi.mock('./components/PublicRoute', () => ({
  default: ({ children }: { children: React.ReactNode }) => (
    <div data-testid="guard-public">{children}</div>
  ),
}));
vi.mock('./components/ProtectedRoute', () => ({
  default: ({ children, requireAuth }: { children: React.ReactNode; requireAuth?: boolean }) => (
    <div data-testid="guard-protected" data-require-auth={String(requireAuth)}>
      {children}
    </div>
  ),
}));
vi.mock('./components/DefaultRedirect', () => ({
  default: () => <div data-testid="default-redirect" />,
}));

// Echo the route params so the `:kind` / `:status` plumbing is observable.
vi.mock('./pages/WebCallbackPage', () => {
  const WebCallbackProbe = ({ callbackKind }: { callbackKind?: string }) => {
    const { kind, status } = useParams();
    return (
      <div data-testid="page-web-callback">
        <span data-testid="callback-kind">{callbackKind ?? kind ?? ''}</span>
        <span data-testid="callback-status">{status ?? ''}</span>
      </div>
    );
  };
  return { default: WebCallbackProbe };
});

vi.mock('./AppRoutesIOS', () => ({ default: () => <div /> }));
vi.mock('./features/human/HumanPage', () => ({ default: () => <div /> }));
vi.mock('./pages/Accounts', () => ({ default: () => <div /> }));
vi.mock('./pages/Brain', () => ({ default: () => <div /> }));
vi.mock('./pages/dev/AgentInsightsPreview', () => ({ default: () => <div /> }));
vi.mock('./pages/dev/UiGallery', () => ({ default: () => <div data-testid="page-ui-gallery" /> }));
vi.mock('./pages/Invites', () => ({ default: () => <div data-testid="page-invites" /> }));
vi.mock('./pages/Notifications', () => ({
  default: () => <div data-testid="page-notifications" />,
}));
vi.mock('./pages/onboarding/Onboarding', () => ({
  default: () => <div data-testid="page-onboarding" />,
}));
vi.mock('./pages/PttOverlayPage', () => ({
  PttOverlayPage: () => <div data-testid="page-ptt-overlay" />,
}));
vi.mock('./pages/Rewards', () => ({ default: () => <div data-testid="page-rewards" /> }));
vi.mock('./pages/Settings', () => ({ default: () => <div /> }));
vi.mock('./pages/Skills', () => ({ default: () => <div /> }));
vi.mock('./pages/Welcome', () => ({ default: () => <div data-testid="page-welcome" /> }));
vi.mock('./pages/WorkflowsRun', () => ({ default: () => <div /> }));

const AppRoutes = (await import('./AppRoutes')).default;

function visit(entry: string) {
  render(
    <MemoryRouter initialEntries={[entry]}>
      <AppRoutes />
    </MemoryRouter>
  );
}

type Guard = 'protected' | 'public' | 'none';
/** `protected-but-open` = wrapped in ProtectedRoute but with the check disabled. */
type TableGuard = Guard | 'redirect' | 'protected-but-open';

/** The route, the page it must render, and the guard that must wrap it. */
const OWNED: Array<{ path: string; page: string; guard: Guard }> = [
  { path: '/', page: 'page-welcome', guard: 'public' },
  { path: '/onboarding/profile', page: 'page-onboarding', guard: 'protected' },
  { path: '/invites', page: 'page-invites', guard: 'protected' },
  { path: '/notifications', page: 'page-notifications', guard: 'protected' },
  { path: '/rewards', page: 'page-rewards', guard: 'protected' },
  { path: '/ptt-overlay', page: 'page-ptt-overlay', guard: 'none' },
  { path: '/dev/ui', page: 'page-ui-gallery', guard: 'none' },
];

describe('AppRoutes — each route renders its page behind the right guard', () => {
  for (const { path, page, guard } of OWNED) {
    it(`${path} renders ${page} with guard=${guard}`, () => {
      visit(path);

      expect(screen.getByTestId(page)).toBeInTheDocument();
      // The catch-all must not have swallowed the route.
      expect(screen.queryByTestId('default-redirect')).not.toBeInTheDocument();

      expect(!!screen.queryByTestId('guard-protected')).toBe(guard === 'protected');
      expect(!!screen.queryByTestId('guard-public')).toBe(guard === 'public');
    });
  }

  it('never disables the check on a route it wraps', () => {
    // `requireAuth` DEFAULTS to true (ProtectedRoute.tsx:18), so omitting it is
    // harmless and this deliberately does not assert the prop is spelled out —
    // that would fail on a harmless tidy-up and catch no real defect. What is
    // dangerous is `requireAuth={false}`: the wrapper is still there, so the
    // route reads as protected in review while the check is off. The route
    // table below classifies that separately, so it cannot pass unnoticed.
    visit('/rewards');
    expect(screen.getByTestId('guard-protected')).not.toHaveAttribute('data-require-auth', 'false');
  });
});

describe('AppRoutes — the OAuth callback family is reachable signed-out', () => {
  // These routes ESTABLISH the session. Wrapping them in ProtectedRoute would
  // make sign-in impossible while every signed-in test still passed.
  it('/auth is unguarded and pins its kind from the prop', () => {
    visit('/auth?token=jwt');

    expect(screen.getByTestId('callback-kind')).toHaveTextContent('auth');
    expect(screen.queryByTestId('guard-protected')).not.toBeInTheDocument();
    expect(screen.queryByTestId('guard-public')).not.toBeInTheDocument();
  });

  it('/callback/:kind takes its kind from the path', () => {
    visit('/callback/composio');

    expect(screen.getByTestId('callback-kind')).toHaveTextContent('composio');
    expect(screen.getByTestId('callback-status')).toBeEmptyDOMElement();
    expect(screen.queryByTestId('guard-protected')).not.toBeInTheDocument();
  });

  it('/callback/:kind/:status carries both segments', () => {
    // The two-segment form is a distinct <Route>. A provider redirecting to
    // `/callback/gmail/success` must not fall through to the catch-all.
    visit('/callback/gmail/success');

    expect(screen.getByTestId('callback-kind')).toHaveTextContent('gmail');
    expect(screen.getByTestId('callback-status')).toHaveTextContent('success');
    expect(screen.queryByTestId('default-redirect')).not.toBeInTheDocument();
  });

  it('routes a failure status to the same handler rather than the catch-all', () => {
    visit('/callback/gmail/error');

    expect(screen.getByTestId('callback-status')).toHaveTextContent('error');
    expect(screen.queryByTestId('default-redirect')).not.toBeInTheDocument();
  });
});

describe('AppRoutes — the whole route table stays classified', () => {
  // Source-level, so it covers routes whose pages are other specs' business.
  // A new route, or a changed guard on any existing one, fails here.
  const EXPECTED: Record<string, TableGuard> = {
    '/': 'public',
    '/auth': 'none',
    '/callback/:kind': 'none',
    '/callback/:kind/:status': 'none',
    '/onboarding/*': 'protected',
    '/home': 'redirect',
    '/human': 'protected',
    '/brain': 'protected',
    '/flows': 'protected',
    '/flows/draft': 'protected',
    '/flows/:id': 'protected',
    '/activity': 'redirect',
    '/intelligence': 'redirect',
    '/workflows/run': 'protected',
    '/connections': 'protected',
    '/skills': 'redirect',
    '/chat/:threadId?': 'protected',
    '/accounts': 'redirect',
    '/channels': 'redirect',
    '/invites': 'protected',
    '/feedback': 'redirect',
    '/notifications': 'protected',
    '/routines': 'redirect',
    '/rewards': 'protected',
    '/workflows': 'protected',
    '/webhooks': 'redirect',
    '/settings/*': 'protected',
    '/ptt-overlay': 'none',
    '/dev/agent-insights': 'none',
    '/dev/ui': 'none',
    '/dev/assistant-ui': 'none',
    '*': 'none',
  };

  function classifyRouteTable(): Record<string, TableGuard> {
    const fs = require('node:fs') as typeof import('node:fs');
    const path = require('node:path') as typeof import('node:path');
    const source = fs.readFileSync(path.resolve('src/AppRoutes.tsx'), 'utf8');

    const out: Record<string, TableGuard> = {};
    // Each `<Route path="X"` up to the start of the next one is that route's body.
    const starts = [...source.matchAll(/<Route\s+path="([^"]+)"/g)];
    starts.forEach((match, index) => {
      const from = match.index ?? 0;
      const to =
        index + 1 < starts.length ? (starts[index + 1].index ?? source.length) : source.length;
      const body = source.slice(from, to);
      // `ForwardSearch` (added by #5924) is a redirect too: it renders a
      // `<Navigate>` internally after copying the query string and hash, so the
      // route body never contains the literal `<Navigate`. Matching only that
      // classified `/skills` as 'none' and silently dropped it from this table.
      if (/<(?:Navigate|ForwardSearch)\b/.test(body)) out[match[1]] = 'redirect';
      else if (/<ProtectedRoute\b[^>]*requireAuth=\{false\}/.test(body))
        out[match[1]] = 'protected-but-open';
      else if (/<ProtectedRoute\b/.test(body)) out[match[1]] = 'protected';
      else if (/<PublicRoute\b/.test(body)) out[match[1]] = 'public';
      else out[match[1]] = 'none';
    });
    return out;
  }

  it('every route carries the guard this table expects', () => {
    expect(classifyRouteTable()).toEqual(EXPECTED);
  });
});
