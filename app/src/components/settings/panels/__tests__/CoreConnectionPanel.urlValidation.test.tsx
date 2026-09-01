/**
 * CoreConnectionPanel — remote-URL validation, and the plain-HTTP warning.
 *
 * The sibling spec drives the save flow with a well-formed
 * `https://core.example.com/rpc` every time, so `validate()`'s rejection arms
 * were never executed (CoreConnectionPanel.tsx lines 195-208) and neither was
 * the public-HTTP warning (225-231). That is most of why the panel sat at
 * 80.18% branch coverage.
 *
 * One of those arms is a credential-leak guard, and it is the reason this file
 * exists. The panel's own comment:
 *
 *   > The separate token field is the only credential path; a
 *   > `user:pass@host` URL would be persisted and echoed back in the
 *   > active-URL description, leaking a secret. Reject it.
 *
 * A regression there does not throw — it persists the password to local
 * storage and renders it back on the settings page. That is exactly the shape
 * of defect a test should hold in place, and it had none.
 */
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';

const hoisted = vi.hoisted(() => ({
  testCoreRpcConnection: vi.fn(),
  clearCoreRpcUrlCache: vi.fn(),
  clearCoreRpcTokenCache: vi.fn(),
  restartApp: vi.fn(),
  isTauriEnvironment: vi.fn(() => true),
  invoke: vi.fn(async () => 'http://127.0.0.1:7788/rpc'),
}));

vi.mock('../../../../services/coreRpcClient', () => ({
  testCoreRpcConnection: hoisted.testCoreRpcConnection,
  clearCoreRpcUrlCache: hoisted.clearCoreRpcUrlCache,
  clearCoreRpcTokenCache: hoisted.clearCoreRpcTokenCache,
}));

vi.mock('../../../../utils/tauriCommands/core', () => ({ restartApp: hoisted.restartApp }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: hoisted.invoke }));

vi.mock('../../../../utils/configPersistence', async importOriginal => {
  const actual = await importOriginal<typeof import('../../../../utils/configPersistence')>();
  return { ...actual, isTauriEnvironment: hoisted.isTauriEnvironment };
});

function okResponse() {
  return { ok: true, status: 200, json: async () => ({ jsonrpc: '2.0', id: 1, result: {} }) };
}

async function importPanel() {
  const mod = await import('../CoreConnectionPanel');
  return mod.default;
}

const URL_ERROR = 'The URL needs to start with http:// or https://';
const INVALID_URL = "That doesn't look like a valid URL (try https://core.example.com/rpc)";
const HTTP_WARNING =
  'This is a plain HTTP URL on a public host: traffic will not be encrypted. Use HTTPS unless you trust this network.';

/** Mount in local mode, flip the remote toggle on, and fill the form. */
async function openRemoteForm(url: string, token = 'remote-token-xyz') {
  hoisted.testCoreRpcConnection.mockResolvedValue(okResponse());
  const Panel = await importPanel();
  const rendered = renderWithProviders(<Panel />, {
    preloadedState: { coreMode: { mode: { kind: 'local' } } },
  });
  await waitFor(() => expect(screen.getByText('Connected to local core')).toBeInTheDocument());

  fireEvent.click(screen.getByTestId('core-use-remote-toggle'));
  fireEvent.change(screen.getByLabelText(/Runtime URL/i), { target: { value: url } });
  fireEvent.change(screen.getByLabelText(/Auth Token/i), { target: { value: token } });
  return rendered;
}

beforeEach(() => {
  vi.resetModules();
  hoisted.testCoreRpcConnection.mockReset();
  hoisted.clearCoreRpcUrlCache.mockReset();
  hoisted.clearCoreRpcTokenCache.mockReset();
  hoisted.restartApp.mockReset();
  hoisted.restartApp.mockResolvedValue(undefined);
  hoisted.isTauriEnvironment.mockReset();
  hoisted.isTauriEnvironment.mockReturnValue(true);
  hoisted.invoke.mockReset();
  hoisted.invoke.mockResolvedValue('http://127.0.0.1:7788/rpc');
  localStorage.clear();
});

describe('CoreConnectionPanel remote URL validation', () => {
  test('rejects a URL carrying credentials rather than persisting the secret', async () => {
    // The guard that matters. Saving must be refused, nothing may reach Redux,
    // and — the part a coverage number would not tell you — the password must
    // not appear anywhere in the rendered page.
    const { store } = await openRemoteForm('https://admin:hunter2@core.example.com/rpc');

    fireEvent.click(screen.getByTestId('core-save-btn'));

    await waitFor(() => expect(screen.getByText(INVALID_URL)).toBeInTheDocument());
    expect(hoisted.restartApp).not.toHaveBeenCalled();
    expect(store.getState().coreMode.mode.kind).toBe('local');
    expect(document.body.textContent).not.toContain('hunter2');
  });

  test('rejects a URL with a username but no password', async () => {
    // `parsed.username || parsed.password` — the username-only half of the
    // guard. Checked separately because a refactor to `&&` would still pass
    // the case above.
    const { store } = await openRemoteForm('https://admin@core.example.com/rpc');

    fireEvent.click(screen.getByTestId('core-save-btn'));

    await waitFor(() => expect(screen.getByText(INVALID_URL)).toBeInTheDocument());
    expect(store.getState().coreMode.mode.kind).toBe('local');
  });

  test('rejects a non-HTTP protocol with the protocol-specific message', async () => {
    // A parseable URL that is not http(s) takes the protocol arm, which has
    // its own copy — asserting the exact string keeps the two arms distinct.
    await openRemoteForm('ftp://core.example.com/rpc');

    fireEvent.click(screen.getByTestId('core-save-btn'));

    await waitFor(() => expect(screen.getByText(URL_ERROR)).toBeInTheDocument());
    expect(hoisted.restartApp).not.toHaveBeenCalled();
  });

  test('rejects an unparseable URL', async () => {
    // The `catch` around `new URL(...)`.
    await openRemoteForm('http://[not a url');

    fireEvent.click(screen.getByTestId('core-save-btn'));

    await waitFor(() => expect(screen.getByText(INVALID_URL)).toBeInTheDocument());
    expect(hoisted.restartApp).not.toHaveBeenCalled();
  });

  test('warns about plain HTTP to a public host', async () => {
    // Advisory, not blocking — the warning renders as you type, before any
    // save. It exists so a user does not silently ship an unencrypted link.
    await openRemoteForm('http://core.example.com/rpc');

    await waitFor(() => expect(screen.getByText(HTTP_WARNING)).toBeInTheDocument());
  });

  test('does not warn about plain HTTP to localhost', async () => {
    // The whole point of `isLocalOrPrivateNetworkHost`: the common, correct
    // case must stay quiet, or the warning becomes noise people learn to skip.
    await openRemoteForm('http://127.0.0.1:7788/rpc');

    await waitFor(() =>
      expect(screen.getByLabelText(/Runtime URL/i)).toHaveValue('http://127.0.0.1:7788/rpc')
    );
    expect(screen.queryByText(HTTP_WARNING)).not.toBeInTheDocument();
  });

  test('does not warn about plain HTTP to a private-network host', async () => {
    await openRemoteForm('http://192.168.1.50:7788/rpc');

    await waitFor(() =>
      expect(screen.getByLabelText(/Runtime URL/i)).toHaveValue('http://192.168.1.50:7788/rpc')
    );
    expect(screen.queryByText(HTTP_WARNING)).not.toBeInTheDocument();
  });

  test('does not warn about HTTPS to a public host', async () => {
    await openRemoteForm('https://core.example.com/rpc');

    await waitFor(() =>
      expect(screen.getByLabelText(/Runtime URL/i)).toHaveValue('https://core.example.com/rpc')
    );
    expect(screen.queryByText(HTTP_WARNING)).not.toBeInTheDocument();
  });
});
