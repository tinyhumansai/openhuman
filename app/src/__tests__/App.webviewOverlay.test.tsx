import { render, screen, waitFor } from '@testing-library/react';
import { useEffect } from 'react';
import { MemoryRouter, useNavigate } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { AppShellDesktop } from '../App';

const { hideWebviewAccountMock, mockDispatch } = vi.hoisted(() => ({
  hideWebviewAccountMock: vi.fn().mockResolvedValue(undefined),
  mockDispatch: vi.fn(),
}));

const baseState = {
  accounts: {
    accounts: {
      'acct-whatsapp': {
        id: 'acct-whatsapp',
        provider: 'whatsapp',
        label: 'WhatsApp',
        createdAt: '2026-01-01T00:00:00.000Z',
        status: 'open',
      },
    },
    order: ['acct-whatsapp'],
    activeAccountId: 'acct-whatsapp',
    lastActiveAccountId: 'acct-whatsapp',
    messages: {},
    unread: {},
    logs: {},
    overlayOpen: false,
  },
  // The desktop shell now mounts <UserErrorCenter/>, which reads this slice
  // via selectActiveUserErrors; include its empty initial shape so the
  // component renders null instead of throwing on an undefined slice (#3931).
  userErrors: { byId: {}, order: [] },
};

let mockState = baseState;

vi.mock('../services/webviewAccountService', () => ({
  hideWebviewAccount: hideWebviewAccountMock,
  startWebviewAccountService: vi.fn(),
  stopWebviewAccountService: vi.fn(),
}));
vi.mock('../lib/webviewNotifications', () => ({
  startWebviewNotificationsService: vi.fn(),
  stopWebviewNotificationsService: vi.fn(),
}));
vi.mock('../lib/nativeNotifications', () => ({
  startNativeNotificationsService: vi.fn(),
  stopNativeNotificationsService: vi.fn(),
}));
vi.mock('../services/internetStatusListener', () => ({
  startInternetStatusListener: vi.fn(),
  stopInternetStatusListener: vi.fn(),
}));
vi.mock('../services/coreHealthMonitor', () => ({
  startCoreHealthMonitor: vi.fn(),
  stopCoreHealthMonitor: vi.fn(),
}));
vi.mock('../services/analytics', () => ({ trackPageView: vi.fn() }));
vi.mock('../hooks/useNotchBootSync', () => ({ useNotchBootSync: vi.fn() }));
vi.mock('../providers/CoreStateProvider', () => ({
  default: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  useCoreState: () => ({
    snapshot: { sessionToken: 'token', onboardingCompleted: true },
    isBootstrapping: false,
  }),
}));
vi.mock('../store/hooks', () => ({
  useAppDispatch: () => mockDispatch,
  useAppSelector: (selector: (state: typeof baseState) => unknown) => selector(mockState),
}));
vi.mock('../components/accounts/WebviewHost', () => ({
  default: ({ accountId }: { accountId: string }) => (
    <div data-testid="webview-host">{accountId}</div>
  ),
}));
vi.mock('../AppRoutes', () => ({ default: () => <main data-testid="routes" /> }));
vi.mock('../components/AppBackground', () => ({ default: () => null }));
vi.mock('../components/layout/shell/AppSidebar', () => ({ default: () => <aside /> }));
vi.mock('../components/layout/shell/RootShellLayout', () => ({
  default: ({ sidebar, children }: { sidebar: React.ReactNode; children: React.ReactNode }) => (
    <div>
      {sidebar}
      {children}
    </div>
  ),
}));
vi.mock('../components/layout/shell/SidebarSlot', () => ({
  SidebarSlotProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
vi.mock('../components/OpenhumanLinkModal', () => ({ default: () => null }));
vi.mock('../components/upsell/GlobalUpsellBanner', () => ({ default: () => null }));
vi.mock('../features/meet/MascotFrameProducer', () => ({ MascotFrameProducer: () => null }));
vi.mock('../components/walkthrough/AppWalkthrough', () => ({ default: () => null }));

describe('AppShellDesktop provider webview visibility', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    HTMLElement.prototype.scrollTo = vi.fn();
    mockState = baseState;
  });

  it('mounts the active provider webview when rail overlays are closed', () => {
    renderShell();

    expect(screen.getByTestId('webview-host')).toHaveTextContent('acct-whatsapp');
  });

  it('does not mount the provider webview while a rail overlay is open', () => {
    mockState = { ...baseState, accounts: { ...baseState.accounts, overlayOpen: true } };

    renderShell();

    expect(screen.queryByTestId('webview-host')).not.toBeInTheDocument();
  });

  it('does not mount a provider webview when the active account is missing', () => {
    mockState = {
      ...baseState,
      accounts: { ...baseState.accounts, activeAccountId: 'missing-account' },
    };

    renderShell();

    expect(screen.queryByTestId('webview-host')).not.toBeInTheDocument();
  });

  it('hides the active provider and restores the agent selection on route changes', async () => {
    renderShell('/chat/thread-1', '/settings');

    await waitFor(() => expect(hideWebviewAccountMock).toHaveBeenCalledWith('acct-whatsapp'));
    expect(mockDispatch).toHaveBeenCalledWith({
      type: 'accounts/setActiveAccount',
      payload: '__agent__',
    });
  });
});

function renderShell(initialPath = '/chat/thread-1', nextPath?: string) {
  return render(
    <MemoryRouter initialEntries={[initialPath]}>
      <RouteChangeHarness nextPath={nextPath} />
    </MemoryRouter>
  );
}

function RouteChangeHarness({ nextPath }: { nextPath?: string }) {
  const navigate = useNavigate();
  useEffect(() => {
    if (nextPath) {
      navigate(nextPath);
    }
  }, [navigate, nextPath]);
  return <AppShellDesktop />;
}
