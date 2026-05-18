/**
 * Tests that App.tsx calls startInternetStatusListener and startCoreHealthMonitor
 * at module boot time (lines 50-51, #1527).
 *
 * We must mock every service/component that App.tsx (or its recursive imports)
 * pulls in at module scope to keep the test fast and isolated.
 */
import { describe, expect, it, vi } from 'vitest';

// ---- Service mocks that must be in place BEFORE App.tsx is imported ----

const startInternetStatusListenerMock = vi.fn();
const stopInternetStatusListenerMock = vi.fn();
const startCoreHealthMonitorMock = vi.fn();
const stopCoreHealthMonitorMock = vi.fn();
const startWebviewAccountServiceMock = vi.fn();
const stopWebviewAccountServiceMock = vi.fn();
const startWebviewNotificationsServiceMock = vi.fn();
const stopWebviewNotificationsServiceMock = vi.fn();
const startNativeNotificationsServiceMock = vi.fn();
const stopNativeNotificationsServiceMock = vi.fn();

vi.mock('../services/internetStatusListener', () => ({
  startInternetStatusListener: startInternetStatusListenerMock,
  stopInternetStatusListener: stopInternetStatusListenerMock,
}));

vi.mock('../services/coreHealthMonitor', () => ({
  startCoreHealthMonitor: startCoreHealthMonitorMock,
  stopCoreHealthMonitor: stopCoreHealthMonitorMock,
}));

// Stub out the heavy services that also run at module boot in App.tsx.
vi.mock('../services/webviewAccountService', () => ({
  startWebviewAccountService: startWebviewAccountServiceMock,
  stopWebviewAccountService: stopWebviewAccountServiceMock,
  isTauri: vi.fn(() => false),
}));
vi.mock('../lib/webviewNotifications', () => ({
  startWebviewNotificationsService: startWebviewNotificationsServiceMock,
  stopWebviewNotificationsService: stopWebviewNotificationsServiceMock,
}));
vi.mock('../lib/nativeNotifications', () => ({
  startNativeNotificationsService: startNativeNotificationsServiceMock,
  stopNativeNotificationsService: stopNativeNotificationsServiceMock,
}));

// Stub out all imports that would pull in Tauri or heavy React trees.
vi.mock('../store', () => ({
  store: { dispatch: vi.fn(), getState: vi.fn(() => ({})), subscribe: vi.fn() },
  persistor: { subscribe: vi.fn(), getState: vi.fn(() => ({ bootstrapped: true })) },
}));
vi.mock('../providers/CoreStateProvider', () => ({
  default: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  useCoreState: vi.fn(() => ({
    snapshot: { sessionToken: null, onboardingCompleted: true },
    isBootstrapping: false,
  })),
}));
vi.mock('../providers/SocketProvider', () => ({
  default: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
vi.mock('../providers/ChatRuntimeProvider', () => ({
  default: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
vi.mock('../AppRoutes', () => ({ default: () => null }));
vi.mock('../components/BootCheckGate/BootCheckGate', () => ({
  default: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
vi.mock('../components/MeshGradient', () => ({ default: () => null }));
vi.mock('../components/BottomTabBar', () => ({ default: () => null }));
vi.mock('../components/AppUpdatePrompt', () => ({ default: () => null }));
vi.mock('../components/LocalAIDownloadSnackbar', () => ({ default: () => null }));
vi.mock('../components/daemon/ServiceBlockingGate', () => ({
  default: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
vi.mock('../components/commands/CommandProvider', () => ({
  default: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
vi.mock('../components/DictationHotkeyManager', () => ({ default: () => null }));
vi.mock('../components/OpenhumanLinkModal', () => ({ default: () => null }));
vi.mock('../components/upsell/GlobalUpsellBanner', () => ({ default: () => null }));
vi.mock('../components/walkthrough/AppWalkthrough', () => ({ default: () => null }));
vi.mock('../features/meet/MascotFrameProducer', () => ({ MascotFrameProducer: () => null }));
vi.mock('../services/analytics', () => ({ trackPageView: vi.fn() }));
vi.mock('../utils/accountsFullscreen', () => ({ isAccountsFullscreen: vi.fn(() => false) }));
vi.mock('../store/hooks', () => ({ useAppSelector: vi.fn(() => null) }));
vi.mock('@sentry/react', () => ({
  ErrorBoundary: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

describe('App.tsx boot-time service wiring (lines 50-51)', () => {
  it('calls startInternetStatusListener and startCoreHealthMonitor at module load', async () => {
    await import('../App');
    expect(startInternetStatusListenerMock).toHaveBeenCalled();
    expect(startCoreHealthMonitorMock).toHaveBeenCalled();
  });

  it('stops boot-time services from the HMR cleanup helper', async () => {
    const { stopBootServicesForHmr } = await import('../App');

    stopBootServicesForHmr();

    expect(stopWebviewAccountServiceMock).toHaveBeenCalled();
    expect(stopWebviewNotificationsServiceMock).toHaveBeenCalled();
    expect(stopNativeNotificationsServiceMock).toHaveBeenCalled();
    expect(stopInternetStatusListenerMock).toHaveBeenCalled();
    expect(stopCoreHealthMonitorMock).toHaveBeenCalled();
  });
});
