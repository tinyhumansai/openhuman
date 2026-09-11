import { render, screen } from '@testing-library/react';
import type React from 'react';
import { MemoryRouter } from 'react-router-dom';
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
vi.mock('./pages/Accounts', () => ({ default: () => <div data-testid="accounts-page" /> }));
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

describe('AppRoutes auth callback aliases', () => {
  it('routes /auth directly to the auth callback handler', () => {
    render(
      <MemoryRouter initialEntries={['/auth?token=jwt-token&key=auth']}>
        <AppRoutes />
      </MemoryRouter>
    );

    expect(screen.getByTestId('web-callback')).toHaveTextContent('auth');
    expect(screen.queryByTestId('default-redirect')).not.toBeInTheDocument();
  });

  it('redirects the retired accounts route to unified chat', () => {
    render(
      <MemoryRouter initialEntries={['/accounts']}>
        <AppRoutes />
      </MemoryRouter>
    );

    expect(screen.getByTestId('accounts-page')).toBeInTheDocument();
  });
});
