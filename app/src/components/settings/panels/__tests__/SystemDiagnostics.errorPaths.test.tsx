/**
 * SystemDiagnostics — the failure paths.
 *
 * The sibling `SystemDiagnostics.test.tsx` covers the happy paths: the Sentry
 * row's visibility per environment, a successful send, the restart-tour row,
 * and the resolved logs-folder path. It leaves every *error* branch
 * unexecuted — measured at 79.48% statements / 57.69% branches, with
 * `SystemDiagnostics.tsx` lines 32, 36-40 and 88 uncovered:
 *
 *   - `:32`    the `logs_folder_path` rejection handler,
 *   - `:36-40` the whole "Open logs folder" click handler, success and failure,
 *   - `:88`    the Sentry send rejection handler.
 *
 * All three surface an operator-facing message, and this panel is the one a
 * user is sent to when something is already broken — so a silent failure here
 * is worse than elsewhere. A new file rather than an edit to the sibling:
 * these need `invoke` to reject per-command, which the sibling's
 * `mockResolvedValue(null)` default would fight.
 */
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';

const hoisted = vi.hoisted(() => ({
  trigger: vi.fn(),
  appEnvironment: 'production' as 'staging' | 'production' | 'development',
  invoke: vi.fn(),
  isTauri: vi.fn(() => true),
  navigate: vi.fn(),
  resetWalkthrough: vi.fn(),
}));

vi.mock('../../../../utils/tauriCommands/common', () => ({
  isTauri: hoisted.isTauri,
  safeInvoke: (...args: unknown[]) => hoisted.invoke(...args),
}));

vi.mock('../../../../services/analytics', () => ({ triggerSentryTestEvent: hoisted.trigger }));

vi.mock('../../../../utils/config', () => ({
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

vi.mock('../../../walkthrough/AppWalkthrough', () => ({
  resetWalkthrough: hoisted.resetWalkthrough,
  setWalkthroughPending: vi.fn(),
}));

vi.mock('react-router-dom', async importOriginal => {
  const actual = await importOriginal<typeof import('react-router-dom')>();
  return { ...actual, useNavigate: () => hoisted.navigate };
});

async function importPanel() {
  const mod = await import('../SystemDiagnostics');
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

describe('<SystemDiagnostics /> failure paths', () => {
  test('surfaces the reason when the logs folder path cannot be resolved', async () => {
    // `:32` — the effect's catch. Without it the row renders with no path and
    // no explanation, which reads as "there are no logs" rather than "we could
    // not look them up".
    hoisted.invoke.mockImplementation((cmd: string) =>
      cmd === 'logs_folder_path'
        ? Promise.reject(new Error('log dir is not readable'))
        : Promise.resolve()
    );
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => expect(screen.getByText('log dir is not readable')).toBeInTheDocument());
    // The row itself must survive the failure — it still offers the button.
    expect(screen.getByRole('button', { name: 'Open logs folder' })).toBeInTheDocument();
  });

  test('stringifies a non-Error rejection from the logs path lookup', async () => {
    // `err instanceof Error ? err.message : String(err)` — a bare string
    // rejection must not render "undefined" into the status line.
    hoisted.invoke.mockImplementation((cmd: string) =>
      cmd === 'logs_folder_path' ? Promise.reject('permission denied') : Promise.resolve()
    );
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => expect(screen.getByText('permission denied')).toBeInTheDocument());
  });

  test('invokes reveal_logs_folder when the open button is clicked', async () => {
    // `:36-40` — the click handler had no test at all.
    hoisted.invoke.mockImplementation((cmd: string) =>
      cmd === 'logs_folder_path' ? Promise.resolve('/tmp/openhuman/logs') : Promise.resolve()
    );
    const Panel = await importPanel();
    renderWithProviders(<Panel />);
    await waitFor(() => expect(screen.getByText('/tmp/openhuman/logs')).toBeInTheDocument());

    fireEvent.click(screen.getByRole('button', { name: 'Open logs folder' }));

    await waitFor(() => expect(hoisted.invoke).toHaveBeenCalledWith('reveal_logs_folder'));
  });

  test('surfaces the reason when revealing the logs folder fails', async () => {
    hoisted.invoke.mockImplementation((cmd: string) => {
      if (cmd === 'logs_folder_path') return Promise.resolve('/tmp/openhuman/logs');
      if (cmd === 'reveal_logs_folder') return Promise.reject(new Error('no file manager'));
      return Promise.resolve();
    });
    const Panel = await importPanel();
    renderWithProviders(<Panel />);
    await waitFor(() => expect(screen.getByText('/tmp/openhuman/logs')).toBeInTheDocument());

    fireEvent.click(screen.getByRole('button', { name: 'Open logs folder' }));

    await waitFor(() => expect(screen.getByText('no file manager')).toBeInTheDocument());
  });

  test('clears a previous reveal error when the button is clicked again', async () => {
    // `setError(null)` is the first line of the handler. Without it a retry
    // that succeeds still shows the stale failure, so the user cannot tell
    // whether the second attempt worked.
    let attempt = 0;
    hoisted.invoke.mockImplementation((cmd: string) => {
      if (cmd === 'logs_folder_path') return Promise.resolve('/tmp/openhuman/logs');
      if (cmd === 'reveal_logs_folder') {
        attempt += 1;
        return attempt === 1 ? Promise.reject(new Error('transient')) : Promise.resolve();
      }
      return Promise.resolve();
    });
    const Panel = await importPanel();
    renderWithProviders(<Panel />);
    await waitFor(() => expect(screen.getByText('/tmp/openhuman/logs')).toBeInTheDocument());

    const button = screen.getByRole('button', { name: 'Open logs folder' });
    fireEvent.click(button);
    await waitFor(() => expect(screen.getByText('transient')).toBeInTheDocument());

    fireEvent.click(button);
    await waitFor(() => expect(screen.queryByText('transient')).not.toBeInTheDocument());
  });

  test('reports a failed Sentry test event instead of claiming it sent', async () => {
    // `:88` — the send's catch. The success path is covered by the sibling
    // file; a rejection previously left the row on its "sending" label with no
    // indication anything went wrong.
    hoisted.appEnvironment = 'staging';
    hoisted.trigger.mockRejectedValue(new Error('DSN rejected the event'));
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    fireEvent.click(screen.getByRole('button', { name: 'Send test event' }));

    await waitFor(() => expect(screen.getByText(/DSN rejected the event/)).toBeInTheDocument());
    expect(screen.queryByText(/Event sent/)).not.toBeInTheDocument();
  });

  test('stringifies a non-Error Sentry rejection', async () => {
    hoisted.appEnvironment = 'staging';
    hoisted.trigger.mockRejectedValue('network down');
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    fireEvent.click(screen.getByRole('button', { name: 'Send test event' }));

    await waitFor(() => expect(screen.getByText(/network down/)).toBeInTheDocument());
  });
});
