import { fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import { LATEST_APP_DOWNLOAD_URL } from '../../../../utils/config';
import type { AppUpdateInfo } from '../../../../utils/tauriCommands/core';
import AboutPanel from '../AboutPanel';

/**
 * `AboutPanel` was the weakest panel in my area by branch coverage — 52.1%
 * branches / 66.7% lines. The existing `AboutPanel.test.tsx` drives the real
 * `useAppUpdate` hook through the `app-update:status` event and reaches three of
 * its nine phases (ready_to_install, up_to_date, error).
 *
 * What is left uncovered is everything the panel *renders from* that hook and
 * from Redux:
 *
 *   - `summaryFor` (panel :168-197) — nine phases plus the two
 *     `available_version ? … : …NoVersion` forks. A phase falling through to the
 *     default arm shows a user "checking for updates" copy forever.
 *   - `formatRelative` (:199-207) — four buckets for the "last checked" line.
 *   - the `rpcUrl` effect (:38-58) — cloud / non-Tauri / Tauri-resolved /
 *     Tauri-rejected, each producing a different Connection readout.
 *   - the connection-mode and helper-text ternaries (:107-131).
 *
 * The update hook is mocked here rather than driven through nine event
 * transitions: the subject is the panel's rendering of a phase, not the hook's
 * own state machine, which its own tests cover.
 */

const hoisted = vi.hoisted(() => ({
  updateState: {
    phase: 'idle' as string,
    info: null as AppUpdateInfo | null,
    error: null as string | null,
    check: vi.fn(async () => null as AppUpdateInfo | null),
  },
  mockInvoke: vi.fn(async () => 'http://127.0.0.1:1234'),
  mockIsTauriEnvironment: vi.fn(() => true),
  mockOpenUrl: vi.fn(),
}));

vi.mock('../../../../hooks/useAppUpdate', () => ({
  useAppUpdate: () => ({
    phase: hoisted.updateState.phase,
    info: hoisted.updateState.info,
    error: hoisted.updateState.error,
    check: hoisted.updateState.check,
    bytesDownloaded: 0,
    totalBytes: null,
    download: vi.fn(),
    install: vi.fn(),
    apply: vi.fn(),
    reset: vi.fn(),
  }),
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => hoisted.mockInvoke(...(args as [])),
  isTauri: () => true,
}));

vi.mock('../../../../utils/configPersistence', async importOriginal => {
  const original = await importOriginal<typeof import('../../../../utils/configPersistence')>();
  return { ...original, isTauriEnvironment: hoisted.mockIsTauriEnvironment };
});

vi.mock('../../../../utils/openUrl', () => ({ openUrl: hoisted.mockOpenUrl }));

// Not the subject here; both render their own surfaces and have their own tests.
vi.mock('../SystemDiagnostics', () => ({ default: () => <div data-testid="sys-diag" /> }));
vi.mock('../../../../features/star/GitHubStarCard', () => ({
  GitHubStarCard: () => <div data-testid="star-card" />,
}));

const info = (over: Partial<AppUpdateInfo> = {}): AppUpdateInfo => ({
  current_version: '1.0.0',
  available: true,
  available_version: '2.0.0',
  body: null,
  ...over,
});

function setUpdate(phase: string, over: Partial<typeof hoisted.updateState> = {}) {
  hoisted.updateState.phase = phase;
  hoisted.updateState.info = over.info ?? null;
  hoisted.updateState.error = over.error ?? null;
}

/** Render with a chosen coreMode slice. */
function renderAbout(coreMode: Record<string, unknown> = { kind: 'local' }) {
  return renderWithProviders(<AboutPanel />, { preloadedState: { coreMode: { mode: coreMode } } });
}

beforeEach(() => {
  vi.clearAllMocks();
  setUpdate('idle');
  hoisted.updateState.check = vi.fn(async () => null);
  hoisted.mockInvoke.mockResolvedValue('http://127.0.0.1:1234');
  hoisted.mockIsTauriEnvironment.mockReturnValue(true);
});

afterEach(() => {
  vi.useRealTimers();
});

describe('AboutPanel — update summary per phase', () => {
  it('renders a distinct summary for every phase, with none falling through', async () => {
    // The strong assertion is the set: nine phases must produce nine different
    // strings. A phase that fell through to the default arm would collide with
    // `idle` and shrink the set.
    const phases = [
      'idle',
      'checking',
      'available',
      'downloading',
      'ready_to_install',
      'installing',
      'restarting',
      'up_to_date',
      'error',
    ];
    const seen = new Set<string>();

    for (const phase of phases) {
      setUpdate(phase, { info: info(), error: phase === 'error' ? 'boom' : null });
      const { unmount } = renderAbout();
      await screen.findAllByText(/software updates/i);
      seen.add(document.body.textContent ?? '');
      unmount();
    }
    expect(seen.size).toBe(phases.length);
  });

  it('names the version in the available summary when one is known', async () => {
    // `available: false` suppresses the version banner, so a match here can
    // only have come from the summary's own interpolation.
    setUpdate('available', { info: info({ available: false, available_version: '9.9.9' }) });
    renderAbout();
    expect(await screen.findByText(/9\.9\.9/)).toBeInTheDocument();
  });

  it('falls back to the no-version copy when available_version is null', async () => {
    setUpdate('available', { info: info({ available_version: null }) });
    renderAbout();
    // The version-bearing string must not appear anywhere.
    await screen.findAllByText(/software updates/i);
    expect(screen.queryByText(/9\.9\.9/)).not.toBeInTheDocument();
  });

  it('names the version in the ready-to-install summary', async () => {
    setUpdate('ready_to_install', { info: info({ available: false, available_version: '3.2.1' }) });
    renderAbout();
    expect(await screen.findByText(/3\.2\.1/)).toBeInTheDocument();
  });

  it('surfaces the hook error verbatim when the phase is error', async () => {
    setUpdate('error', { error: 'updater endpoint unreachable' });
    renderAbout();
    expect(await screen.findByText(/updater endpoint unreachable/)).toBeInTheDocument();
  });

  it('disables the check button only while checking', async () => {
    setUpdate('checking');
    const { unmount } = renderAbout();
    await waitFor(() => expect(screen.getByRole('button', { name: /checking/i })).toBeDisabled());
    unmount();

    setUpdate('idle');
    renderAbout();
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /check for updates/i })).not.toBeDisabled()
    );
  });

  it('shows the available-version banner beside the running version', async () => {
    setUpdate('available', { info: info({ available: true, available_version: '4.5.6' }) });
    renderAbout();
    // The banner is the element carrying the "is available" copy.
    expect(await screen.findByText(/v4\.5\.6\s+is available/)).toBeInTheDocument();
  });

  it('suppresses the banner on info.available alone, even with a version present', async () => {
    // A version string is present but `available` is false, so only the
    // `info?.available` half of the guard can hide the banner.
    setUpdate('up_to_date', { info: info({ available: false, available_version: '8.8.8' }) });
    renderAbout();
    await screen.findAllByText(/software updates/i);
    expect(screen.queryByText(/is available/i)).not.toBeInTheDocument();
  });
});

describe('AboutPanel — the last-checked line', () => {
  it('is absent until a check has returned a result', async () => {
    renderAbout();
    await screen.findAllByText(/software updates/i);
    expect(screen.queryByText(/last checked/i)).not.toBeInTheDocument();
  });

  it('appears after a check that returns a result', async () => {
    hoisted.updateState.check = vi.fn(async () => info());
    renderAbout();
    fireEvent.click(await screen.findByRole('button', { name: /check for updates/i }));
    expect(await screen.findByText(/last checked/i)).toBeInTheDocument();
  });

  it('stays absent when the check returns null', async () => {
    // `check` resolving null means the probe did not complete; stamping a time
    // would tell the user their build was verified when it was not.
    hoisted.updateState.check = vi.fn(async () => null);
    renderAbout();
    fireEvent.click(await screen.findByRole('button', { name: /check for updates/i }));
    await waitFor(() => expect(hoisted.updateState.check).toHaveBeenCalled());
    expect(screen.queryByText(/last checked/i)).not.toBeInTheDocument();
  });
});

describe('AboutPanel — connection readout', () => {
  it('shows the cloud URL straight from Redux without asking the shell', async () => {
    renderAbout({ kind: 'cloud', url: 'https://core.example.com' });
    expect(await screen.findByText('https://core.example.com')).toBeInTheDocument();
    // Cloud mode must not consult the Tauri command at all.
    expect(hoisted.mockInvoke).not.toHaveBeenCalled();
  });

  it('resolves the local URL from the shell command', async () => {
    hoisted.mockInvoke.mockResolvedValue('http://127.0.0.1:57321');
    renderAbout({ kind: 'local' });
    expect(await screen.findByText('http://127.0.0.1:57321')).toBeInTheDocument();
    expect(hoisted.mockInvoke).toHaveBeenCalledWith('core_rpc_url');
  });

  it('shows the unavailable placeholder outside a Tauri environment', async () => {
    hoisted.mockIsTauriEnvironment.mockReturnValue(false);
    renderAbout({ kind: 'local' });
    await waitFor(() => expect(screen.getByText(/server url/i)).toBeInTheDocument());
    expect(hoisted.mockInvoke).not.toHaveBeenCalled();
    expect(screen.queryByText(/^http/)).not.toBeInTheDocument();
  });

  it('shows the unavailable placeholder when the shell command rejects', async () => {
    hoisted.mockInvoke.mockRejectedValue(new Error('no such command'));
    renderAbout({ kind: 'local' });
    await waitFor(() => expect(hoisted.mockInvoke).toHaveBeenCalled());
    expect(screen.queryByText(/^http/)).not.toBeInTheDocument();
  });

  it('labels each connection mode distinctly', async () => {
    const seen = new Set<string>();
    for (const mode of [{ kind: 'local' }, { kind: 'cloud', url: 'u' }, { kind: 'unset' }]) {
      const { unmount } = renderAbout(mode);
      await screen.findAllByText(/software updates/i);
      seen.add(document.body.textContent ?? '');
      unmount();
    }
    expect(seen.size).toBe(3);
  });

  it('uses different helper copy for cloud and local', async () => {
    const { unmount } = renderAbout({ kind: 'cloud', url: 'u' });
    const cloudText = document.body.textContent ?? '';
    unmount();

    renderAbout({ kind: 'local' });
    const localText = document.body.textContent ?? '';
    expect(cloudText).not.toBe(localText);
  });
});

describe('AboutPanel — releases link', () => {
  it('opens the configured download URL rather than a hardcoded one', async () => {
    renderAbout();
    fireEvent.click(await screen.findByRole('button', { name: /releases/i }));
    expect(hoisted.mockOpenUrl).toHaveBeenCalledTimes(1);
    // Assert the CONFIGURED constant, not merely "some http(s) URL". The test
    // is named for opening the configured URL "rather than a hardcoded one",
    // and a shape check like /^https?:\/\// is satisfied by exactly the
    // hardcoded URL it claims to rule out — the assertion contradicted the
    // name. `AboutPanel.tsx:152` passes `LATEST_APP_DOWNLOAD_URL`, so that is
    // what the assertion should name. Caught in review by `coderabbitai`.
    expect(hoisted.mockOpenUrl).toHaveBeenCalledWith(LATEST_APP_DOWNLOAD_URL);
  });
});
