import { render, screen, waitFor } from '@testing-library/react';
import { useEffect } from 'react';
import { MemoryRouter, useNavigate } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { AppShellDesktop } from '../App';

const { hideWebviewAccountMock, mockDispatch, webviewMountSpy, webviewUnmountSpy } = vi.hoisted(
  () => ({
    hideWebviewAccountMock: vi.fn().mockResolvedValue(undefined),
    mockDispatch: vi.fn(),
    // Fired from the mocked WebviewHost's mount/unmount only (empty-deps
    // effect) so a real remount is observable and distinguishable from a
    // same-instance prop update. See the #4421 regression test below.
    webviewMountSpy: vi.fn(),
    webviewUnmountSpy: vi.fn(),
  })
);

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
vi.mock('../components/accounts/WebviewHost', () => {
  // Empty deps: the spies fire only on a genuine mount/unmount, not on a prop
  // change. If App reuses one host instance across account switches (no `key`),
  // switching accounts is a prop update and neither spy fires for the new
  // account — which is exactly the desync #4421 guards against.
  const MockWebviewHost = ({ accountId }: { accountId: string }) => {
    useEffect(() => {
      webviewMountSpy(accountId);
      return () => webviewUnmountSpy(accountId);
      // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);
    return <div data-testid="webview-host">{accountId}</div>;
  };
  return { default: MockWebviewHost };
});
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

  it('remounts a fresh host when the active provider account changes — no cross-account webview bleed (#4421)', () => {
    mockState = stateWithActive('acct-whatsapp');
    const { rerender } = renderShell();

    expect(webviewMountSpy).toHaveBeenCalledWith('acct-whatsapp');
    expect(screen.getByTestId('webview-host')).toHaveTextContent('acct-whatsapp');

    // Rapid rail switch to a different provider account. With `key={id}` on
    // <WebviewHost>, React tears down the WhatsApp host (unmount) and mounts a
    // fresh Slack host rather than reusing the instance with new props — so the
    // previous provider's webview can't linger in the new account's slot.
    mockState = stateWithActive('acct-slack');
    rerender(
      <MemoryRouter initialEntries={['/chat/thread-1']}>
        <RouteChangeHarness />
      </MemoryRouter>
    );

    expect(webviewUnmountSpy).toHaveBeenCalledWith('acct-whatsapp');
    expect(webviewMountSpy).toHaveBeenCalledWith('acct-slack');
    expect(screen.getByTestId('webview-host')).toHaveTextContent('acct-slack');
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

function stateWithActive(activeAccountId: string) {
  return {
    ...baseState,
    accounts: {
      ...baseState.accounts,
      accounts: {
        ...baseState.accounts.accounts,
        'acct-slack': {
          id: 'acct-slack',
          provider: 'slack',
          label: 'Slack',
          createdAt: '2026-01-01T00:00:00.000Z',
          status: 'open',
        },
      },
      activeAccountId,
    },
  };
}

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
