import { render, screen } from '@testing-library/react';
import type React from 'react';
import { MemoryRouter, useLocation } from 'react-router-dom';
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

vi.mock('./pages/WebCallbackPage', () => ({
  default: ({ callbackKind }: { callbackKind?: string }) => (
    <div data-testid="web-callback">{callbackKind ?? 'route-param'}</div>
  ),
}));

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
vi.mock('./pages/Skills', () => ({ default: () => <div data-testid="skills-page" /> }));
vi.mock('./pages/Welcome', () => ({ default: () => <div /> }));
vi.mock('./pages/WorkflowsRun', () => ({ default: () => <div /> }));

// Reads the ambient router location and passes it to a callback. Rendered as a
// sibling of AppRoutes so it sees the settled location after any Navigate fires.
function LocationSpy({
  onCapture,
}: {
  onCapture: (loc: { pathname: string; search: string; hash: string }) => void;
}) {
  const { pathname, search, hash } = useLocation();
  onCapture({ pathname, search, hash });
  return null;
}

const AppRoutes = (await import('./AppRoutes')).default;

describe('/skills back-compat redirect', () => {
  it('redirects /skills to /connections (Skills page renders)', () => {
    render(
      <MemoryRouter initialEntries={['/skills']}>
        <AppRoutes />
      </MemoryRouter>
    );
    expect(screen.getByTestId('skills-page')).toBeInTheDocument();
  });

  it('forwards ?tab= query params from /skills to /connections', () => {
    // Object.assign so the last render-pass wins (initial /skills is overwritten
    // by the settled /connections after Navigate fires via useLayoutEffect).
    const loc = { pathname: '', search: '', hash: '' };
    render(
      <MemoryRouter initialEntries={['/skills?tab=mcp']}>
        <AppRoutes />
        <LocationSpy onCapture={l => Object.assign(loc, l)} />
      </MemoryRouter>
    );
    expect(loc.pathname).toBe('/connections');
    expect(loc.search).toBe('?tab=mcp');
  });

  it('forwards a hash fragment from /skills to /connections', () => {
    const loc = { pathname: '', search: '', hash: '' };
    render(
      <MemoryRouter initialEntries={['/skills#section-mcp']}>
        <AppRoutes />
        <LocationSpy onCapture={l => Object.assign(loc, l)} />
      </MemoryRouter>
    );
    expect(loc.pathname).toBe('/connections');
    expect(loc.hash).toBe('#section-mcp');
  });

  it('produces an empty search when /skills has no query string', () => {
    const loc = { pathname: '', search: '(init)', hash: '' };
    render(
      <MemoryRouter initialEntries={['/skills']}>
        <AppRoutes />
        <LocationSpy onCapture={l => Object.assign(loc, l)} />
      </MemoryRouter>
    );
    expect(loc.pathname).toBe('/connections');
    expect(loc.search).toBe('');
  });
});
