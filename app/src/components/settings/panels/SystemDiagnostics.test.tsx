/**
 * Tests for SystemDiagnostics — the app-logs / staging Sentry-test /
 * restart-tour rows shown on the About page. Mirrors the mock shape used by
 * `__tests__/DeveloperOptionsPanel.test.tsx` for the same underlying rows.
 */
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';

const hoisted = vi.hoisted(() => ({
  trigger: vi.fn(),
  appEnvironment: 'production' as 'staging' | 'production' | 'development',
  invoke: vi.fn(),
  isTauri: vi.fn(() => true),
  navigate: vi.fn(),
  resetWalkthrough: vi.fn(),
}));

vi.mock('../../../utils/tauriCommands/common', () => ({
  isTauri: hoisted.isTauri,
  safeInvoke: (...args: unknown[]) => hoisted.invoke(...args),
}));

vi.mock('../../../services/analytics', () => ({ triggerSentryTestEvent: hoisted.trigger }));

vi.mock('../../../utils/config', () => ({
  APP_BINARY_VERSION: '0.0.0-test',
  get APP_ENVIRONMENT() {
    return hoisted.appEnvironment;
  },
  APP_VERSION: '0.0.0-test',
  BUILD_SHA: 'test',
  CORE_CARGO_VERSION: '0.0.0-test',
  GA_MEASUREMENT_ID: undefined,
  IS_DEV: true,
  OPENPANEL_API_URL: 'https://panel.tinyhumans.ai/api',
  OPENPANEL_CLIENT_ID: undefined,
  SENTRY_DSN: undefined,
  SENTRY_RELEASE: 'openhuman@test',
  SENTRY_SMOKE_TEST: false,
  TAURI_CARGO_VERSION: '0.0.0-test',
  CORE_RPC_URL: 'http://127.0.0.1:7788/rpc',
  BACKEND_URL: 'http://localhost:5005',
  E2E_DEFAULT_CORE_MODE: '',
}));

vi.mock('../../walkthrough/AppWalkthrough', () => ({
  resetWalkthrough: hoisted.resetWalkthrough,
  setWalkthroughPending: vi.fn(),
}));

vi.mock('react-router-dom', async importOriginal => {
  const actual = await importOriginal<typeof import('react-router-dom')>();
  return { ...actual, useNavigate: () => hoisted.navigate };
});

async function importPanel() {
  const mod = await import('./SystemDiagnostics');
  return mod.default;
}

beforeEach(() => {
  vi.resetModules();
  hoisted.invoke.mockReset();
  hoisted.invoke.mockResolvedValue(null);
  hoisted.isTauri.mockReset();
  hoisted.isTauri.mockReturnValue(true);
  hoisted.trigger.mockReset();
  hoisted.navigate.mockReset();
  hoisted.resetWalkthrough.mockReset();
  hoisted.appEnvironment = 'production';
});

describe('<SystemDiagnostics />', () => {
  test('does not render the Sentry test row in production builds', async () => {
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    expect(screen.queryByRole('button', { name: /Send test event/i })).toBeNull();
  });

  test('renders the Sentry test row in staging and fires the helper on click', async () => {
    hoisted.appEnvironment = 'staging';
    hoisted.trigger.mockResolvedValue('event-id-xyz');
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    const button = screen.getByRole('button', { name: /Send test event/i });
    fireEvent.click(button);

    await waitFor(() => expect(hoisted.trigger).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(screen.getByText(/event-id-xyz/)).toBeInTheDocument());
  });

  test('resets the walkthrough and navigates home from the restart-tour row', async () => {
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    fireEvent.click(screen.getByRole('button', { name: /Restart tour/i }));

    expect(hoisted.resetWalkthrough).toHaveBeenCalledTimes(1);
    expect(hoisted.navigate).toHaveBeenCalledWith('/home');
  });

  test('renders the resolved logs folder path under Tauri', async () => {
    hoisted.invoke.mockImplementation((cmd: string) => {
      if (cmd === 'logs_folder_path') return Promise.resolve('/tmp/openhuman/logs');
      return Promise.resolve();
    });
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => expect(screen.getByText('/tmp/openhuman/logs')).toBeInTheDocument());
  });
});
