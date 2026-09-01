/**
 * Route-table coverage for the connections / channels / flows / automation
 * surfaces.
 *
 * WHY THIS FILE EXISTS, given `pages/__tests__/Connections.redirects.test.tsx`
 * already claims to cover two of these redirects:
 *
 * That file declares its **own** local `<TestRoutes>` copy of three routes and
 * renders that, so it asserts React Router's `<Navigate>` works rather than
 * asserting anything about this app's route table. Deleting `/skills` from
 * `AppRoutes.tsx` leaves it green. It also never inspects the landing URL,
 * which is the entire payload of the `/channels` redirect.
 *
 * This file mounts the REAL `AppRoutes` (same mocking pattern as
 * `AppRoutes.auth.test.tsx`) and asserts the landing `pathname + search`, so a
 * change to the route table is what makes it fail.
 */
import { render, screen } from '@testing-library/react';
import type React from 'react';
import { MemoryRouter, useLocation, useParams } from 'react-router-dom';
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

vi.mock('./AppRoutesIOS', () => ({ default: () => <div /> }));
vi.mock('./features/human/HumanPage', () => ({ default: () => <div /> }));
vi.mock('./pages/Accounts', () => ({ default: () => <div /> }));
vi.mock('./pages/Brain', () => ({ default: () => <div /> }));
vi.mock('./pages/dev/AgentInsightsPreview', () => ({ default: () => <div /> }));
vi.mock('./pages/dev/assistant-ui-demo', () => ({ default: () => <div /> }));
vi.mock('./pages/dev/UiGallery', () => ({ default: () => <div /> }));
vi.mock('./pages/Invites', () => ({ default: () => <div /> }));
vi.mock('./pages/Notifications', () => ({ default: () => <div /> }));
vi.mock('./pages/onboarding/Onboarding', () => ({ default: () => <div /> }));
vi.mock('./pages/PttOverlayPage', () => ({ PttOverlayPage: () => <div /> }));
vi.mock('./pages/Rewards', () => ({ default: () => <div /> }));
vi.mock('./pages/Settings', () => ({ default: () => <div /> }));
vi.mock('./pages/Welcome', () => ({ default: () => <div /> }));
vi.mock('./pages/WebCallbackPage', () => ({ default: () => <div /> }));

// The pages this file actually asserts on. Each renders a probe that reports
// the landing location, so an assertion failure names the URL we landed on.
vi.mock('./pages/Skills', () => ({ default: () => <div data-testid="page">connections</div> }));
vi.mock('./pages/FlowsPage', () => ({ default: () => <div data-testid="page">flows</div> }));
vi.mock('./pages/Activity', () => ({ default: () => <div data-testid="page">activity</div> }));
vi.mock('./pages/WorkflowsRun', () => ({
  default: () => <div data-testid="page">workflows-run</div>,
}));
vi.mock('./pages/FlowCanvasPage', () => ({
  // Named + capitalised so eslint's rules-of-hooks recognises it as a component;
  // an anonymous arrow assigned to `default` is not, and `useParams` then trips
  // "called in function 'default' that is neither a component nor a custom Hook".
  default: function FlowCanvasPageMock() {
    const { id } = useParams();
    return <div data-testid="page">{`flow-canvas:${id ?? ''}`}</div>;
  },
  FlowCanvasDraftPage: () => <div data-testid="page">flow-canvas-draft</div>,
}));

const AppRoutes = (await import('./AppRoutes')).default;

/** Reports the live location so assertions can name the URL we landed on. */
function LocationProbe() {
  const loc = useLocation();
  // `hash` included deliberately. Without it the probe reports only
  // `pathname + search`, so a route whose destination carries a fragment —
  // AGENTS.md:175 specifies `/webhooks` -> `/settings/integrations#webhooks` —
  // could lose that fragment and every assertion here would stay green
  // (#5883, Codex). This is a HashRouter, so `loc.hash` is the fragment WITHIN
  // the routed path, not the leading `#/` of the URL.
  return <span data-testid="href">{loc.pathname + loc.search + loc.hash}</span>;
}

function renderAt(entry: string) {
  render(
    <MemoryRouter initialEntries={[entry]}>
      <LocationProbe />
      <AppRoutes />
    </MemoryRouter>
  );
  return {
    href: () => screen.getByTestId('href').textContent,
    page: () => screen.queryByTestId('page')?.textContent,
  };
}

describe('connections / channels back-compat redirects (real route table)', () => {
  it('/skills lands on /connections and renders the Connections page', () => {
    const at = renderAt('/skills');
    expect(at.href()).toBe('/connections');
    expect(at.page()).toBe('connections');
  });

  it('/channels lands on /connections?tab=messaging, preserving the tab selector', () => {
    // The whole point of this redirect: `/channels` was an orphaned standalone
    // page, and the messaging tab of Connections replaced it. Landing on bare
    // `/connections` would drop the user on the Welcome tab instead — which is
    // exactly what `Connections.redirects.test.tsx` cannot distinguish, because
    // it only asserts that the page rendered.
    const at = renderAt('/channels');
    expect(at.href()).toBe('/connections?tab=messaging');
    expect(at.page()).toBe('connections');
  });

  it('/skills forwards its ?tab= query through to /connections', () => {
    // This used to pin the opposite: `<Navigate to="/connections" />` was given
    // an absolute path *string* with no search component, so the incoming query
    // was discarded — while `AppRoutes.tsx` claimed twice that the redirect
    // "preserves ?tab= deep links".
    //
    // openhuman#5924 replaced it with `<ForwardSearch to="/connections" />`,
    // which reads `useLocation()` and copies both `search` and `hash` onto the
    // destination. The two source comments are now true, so they were left
    // alone exactly as the previous revision of this test asked.
    //
    // The knock-on matters more than the redirect: `pages/Skills.tsx`'s legacy
    // alias table (`apps`→`composio`, `messaging`→`channels`, `tools`→`mcp`,
    // `explorer`→`skills`) exists "so that e.g. `/skills?tab=composio` still
    // works after the redirect". While the query was dropped it was unreachable
    // dead code; this assertion is what proves the route can reach it at all.
    const at = renderAt('/skills?tab=messaging');
    expect(at.href()).toBe('/connections?tab=messaging');
  });
});

describe('automation route slugs', () => {
  it('/routines redirects to /flows', () => {
    // `AGENTS.md:175` documents `/routines` -> `/settings/automations`. The two
    // agree on where the user ends up: `/settings/automations` is itself
    // `<Navigate to="/flows">` (settingsRouteElements.tsx:164), so the code
    // short-circuits one hop of the documented chain and lands in the same
    // place. `/workflows` is the real divergence — see the test below.
    const at = renderAt('/routines');
    expect(at.href()).toBe('/flows');
    expect(at.page()).toBe('flows');
  });

  it('/webhooks redirects to the Integrations settings page', () => {
    // Webhooks were retired from the UI; the route survives only to keep old
    // deep links from 404-ing.
    //
    // ⚠️ CONTRACT DIVERGENCE. `AGENTS.md:175` is the authoritative route table
    // and specifies `/webhooks` -> `/settings/integrations#webhooks`. The code
    // (`AppRoutes.tsx`) emits no fragment, so the Webhooks section is not
    // selected on arrival. This asserts what the app DOES; changing it to the
    // documented destination would be a knowingly-failing test. The fix is a
    // source change and someone has to decide which side moves — see W5 BUG-13.
    const at = renderAt('/webhooks');
    expect(at.href()).toBe('/settings/integrations');
  });

  it('/workflows is NOT a redirect — it renders the legacy SKILL.md hub', () => {
    // ⚠️ CONTRACT DIVERGENCE, and a three-way one. `AGENTS.md:175` says
    // `/workflows` -> `/settings/automations`; `AppRoutes.tsx`'s own `/flows`
    // block comment says it redirects to `/flows`; the code renders <Activity/>
    // and stays put. Three statements, three destinations. This pins the code.
    // See W5 BUG-13 (#5883, Codex).
    // Guards against the stale claim in `AppRoutes.tsx`'s own `/flows` block
    // comment, which says "the bare `/workflows` and `/routines` slugs now
    // redirect here (to `/flows`)". Only `/routines` does. `/workflows` renders
    // Activity, per the comment directly above its own route.
    const at = renderAt('/workflows');
    expect(at.href()).toBe('/workflows');
    expect(at.page()).toBe('activity');
  });

  it('/workflows/run renders the single-purpose Skill runner, not the hub', () => {
    const at = renderAt('/workflows/run');
    expect(at.href()).toBe('/workflows/run');
    expect(at.page()).toBe('workflows-run');
  });
});

describe('/flows canvas route ranking', () => {
  it('/flows renders the flows list hub', () => {
    expect(renderAt('/flows').page()).toBe('flows');
  });

  it('/flows/draft resolves to the draft canvas, not to /flows/:id', () => {
    // `AppRoutes.tsx` warns that if `:id` won this match, the canvas would call
    // `flows_get('draft')` for a flow that does not exist. Pin the resolution
    // so a reorder or a rename of either route is caught here.
    const at = renderAt('/flows/draft');
    expect(at.page()).toBe('flow-canvas-draft');
    expect(at.page()).not.toBe('flow-canvas:draft');
  });

  it('/flows/:id resolves to the canvas and hands it the id', () => {
    expect(renderAt('/flows/flow_abc123').page()).toBe('flow-canvas:flow_abc123');
  });
});
