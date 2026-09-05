import { render, screen } from '@testing-library/react';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';

vi.mock('./lib/platform', () => ({ getIsMobile: () => false }));
vi.mock('./AppRoutesIOS', () => ({ default: () => <div data-testid="ios-routes">ios</div> }));
vi.mock('./components/DefaultRedirect', () => ({
  default: () => <div data-testid="default-redirect">redirect</div>,
}));
vi.mock('./components/ProtectedRoute', () => ({
  default: ({ children, requireAuth }: { children: ReactNode; requireAuth?: boolean }) => (
    <div data-testid="protected-route" data-require-auth={String(requireAuth)}>
      {children}
    </div>
  ),
}));
vi.mock('./components/PublicRoute', () => ({
  default: ({ children }: { children: ReactNode }) => (
    <div data-testid="public-route">{children}</div>
  ),
}));
vi.mock('./features/human/HumanPage', () => ({
  default: () => <div data-testid="page-human">human</div>,
}));
vi.mock('./features/coreRegistries/CoreRegistriesPage', () => ({
  default: () => <div data-testid="page-core-registries">core-registries</div>,
}));
vi.mock('./pages/Accounts', () => ({
  default: () => <div data-testid="page-accounts">accounts</div>,
}));
vi.mock('./components/routing/ForwardSearch', () => ({
  default: ({ to }: { to: string }) => <div data-testid={`forward:${to}`}>{to}</div>,
}));
vi.mock('./pages/Activity', () => ({
  default: () => <div data-testid="page-activity">activity</div>,
}));
vi.mock('./pages/Brain', () => ({ default: () => <div data-testid="page-brain">brain</div> }));
vi.mock('./pages/dev/AgentInsightsPreview', () => ({
  default: () => <div data-testid="page-agent-insights">agent-insights</div>,
}));
vi.mock('./pages/dev/assistant-ui-demo', () => ({
  default: () => <div data-testid="page-assistant-ui-demo">assistant-ui-demo</div>,
}));
vi.mock('./pages/dev/UiGallery', () => ({
  default: () => <div data-testid="page-ui-gallery">ui-gallery</div>,
}));
vi.mock('./pages/FlowCanvasPage', () => ({
  default: () => <div data-testid="page-flow-canvas">flow-canvas</div>,
  FlowCanvasDraftPage: () => <div data-testid="page-flow-canvas-draft">flow-canvas-draft</div>,
}));
vi.mock('./pages/FlowsPage', () => ({ default: () => <div data-testid="page-flows">flows</div> }));
vi.mock('./pages/Home', () => ({ default: () => <div data-testid="page-home">home</div> }));
vi.mock('./pages/Invites', () => ({
  default: () => <div data-testid="page-invites">invites</div>,
}));
vi.mock('./pages/Notifications', () => ({
  default: () => <div data-testid="page-notifications">notifications</div>,
}));
vi.mock('./pages/onboarding/Onboarding', () => ({
  default: () => <div data-testid="page-onboarding">onboarding</div>,
}));
vi.mock('./pages/PttOverlayPage', () => ({
  PttOverlayPage: () => <div data-testid="page-ptt-overlay">ptt-overlay</div>,
}));
vi.mock('./pages/Rewards', () => ({
  default: () => <div data-testid="page-rewards">rewards</div>,
}));
vi.mock('./pages/Settings', () => ({
  default: () => <div data-testid="page-settings">settings</div>,
}));
vi.mock('./pages/Skills', () => ({ default: () => <div data-testid="page-skills">skills</div> }));
vi.mock('./pages/WebCallbackPage', () => ({
  default: () => <div data-testid="page-web-callback">callback</div>,
}));
vi.mock('./pages/Welcome', () => ({
  default: () => <div data-testid="page-welcome">welcome</div>,
}));
vi.mock('./pages/Workbench', () => ({
  default: () => <div data-testid="page-workbench">workbench</div>,
}));
vi.mock('./pages/ActionRequestInbox', () => ({
  default: () => <div data-testid="page-action-request-inbox">action-requests</div>,
}));
vi.mock('./pages/WorkflowsRun', () => ({
  default: () => <div data-testid="page-workflows-run">workflows-run</div>,
}));

const AppRoutes = (await import('./AppRoutes')).default;

describe('AppRoutes', () => {
  it('registers the Home route behind the protected desktop shell', () => {
    render(
      <MemoryRouter initialEntries={['/home']}>
        <AppRoutes />
      </MemoryRouter>
    );

    expect(screen.getByTestId('page-home')).toBeInTheDocument();
    expect(screen.getByTestId('protected-route')).toHaveAttribute('data-require-auth', 'true');
  });

  it('registers the Core Registries route behind the protected desktop shell', () => {
    render(
      <MemoryRouter initialEntries={['/registries']}>
        <AppRoutes />
      </MemoryRouter>
    );

    expect(screen.getByTestId('page-core-registries')).toBeInTheDocument();
    expect(screen.getByTestId('protected-route')).toHaveAttribute('data-require-auth', 'true');
  });

  it('preserves the Workbench route behind the protected desktop shell', () => {
    render(
      <MemoryRouter initialEntries={['/workbench']}>
        <AppRoutes />
      </MemoryRouter>
    );

    expect(screen.getByTestId('page-workbench')).toBeInTheDocument();
    expect(screen.getByTestId('protected-route')).toHaveAttribute('data-require-auth', 'true');
  });

  it('preserves the ActionRequest inbox behind the protected desktop shell', () => {
    render(
      <MemoryRouter initialEntries={['/action-requests']}>
        <AppRoutes />
      </MemoryRouter>
    );

    expect(screen.getByTestId('page-action-request-inbox')).toBeInTheDocument();
    expect(screen.getByTestId('protected-route')).toHaveAttribute('data-require-auth', 'true');
  });
});
