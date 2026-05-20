import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';

// [composio-direct] Co-located unit tests for ComposioPanel.tsx — covers
// the mode toggle, the conditional API-key field, the persistence wiring,
// the password-mask post-save, and the clear-key fallback when the user
// switches back to Backend mode.

const hoisted = vi.hoisted(() => ({ getMode: vi.fn(), setApiKey: vi.fn(), clearApiKey: vi.fn() }));

vi.mock('../../../../utils/tauriCommands', () => ({
  openhumanComposioGetMode: hoisted.getMode,
  openhumanComposioSetApiKey: hoisted.setApiKey,
  openhumanComposioClearApiKey: hoisted.clearApiKey,
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    navigateToSettings: vi.fn(),
    breadcrumbs: [],
  }),
}));

vi.mock('../../components/SettingsHeader', () => ({
  default: ({ title }: { title: string }) => <div data-testid="settings-header">{title}</div>,
}));

async function importPanel() {
  vi.resetModules();
  const mod = await import('../ComposioPanel');
  return mod.default;
}

const backendMode = { result: { mode: 'backend' as const, api_key_set: false }, logs: [] };

const directModeWithKey = { result: { mode: 'direct' as const, api_key_set: true }, logs: [] };

describe('ComposioPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    hoisted.getMode.mockResolvedValue(backendMode);
    hoisted.setApiKey.mockResolvedValue({ result: { stored: true, mode: 'direct' }, logs: [] });
    hoisted.clearApiKey.mockResolvedValue({ result: { cleared: true, mode: 'backend' }, logs: [] });
  });

  test('shows loading state then renders header', async () => {
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    expect(screen.getByText('Loading…')).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.queryByText('Loading…')).toBeNull();
    });
    expect(screen.getByText('Composio')).toBeInTheDocument();
  });

  test('defaults to Backend mode when getMode returns backend', async () => {
    const Panel = await importPanel();
    renderWithProviders(<Panel />);
    await waitFor(() => expect(screen.queryByText('Loading…')).toBeNull());

    const backendRadio = screen.getByLabelText(
      'Managed (OpenHuman handles it for you)'
    ) as HTMLInputElement;
    const directRadio = screen.getByLabelText(
      'Direct (bring your own API key)'
    ) as HTMLInputElement;

    expect(backendRadio.checked).toBe(true);
    expect(directRadio.checked).toBe(false);
    // Key field hidden when backend is active.
    expect(screen.queryByLabelText('Composio API key')).toBeNull();
  });

  test('reflects direct mode and stored-key flag from backend payload', async () => {
    hoisted.getMode.mockResolvedValue(directModeWithKey);
    const Panel = await importPanel();
    renderWithProviders(<Panel />);
    await waitFor(() => expect(screen.queryByText('Loading…')).toBeNull());

    const directRadio = screen.getByLabelText(
      'Direct (bring your own API key)'
    ) as HTMLInputElement;
    expect(directRadio.checked).toBe(true);
    // Key field is visible.
    expect(screen.getByLabelText('Composio API key')).toBeInTheDocument();
    // "Key currently stored" indicator shows.
    expect(
      screen.getByText('A Composio API key is currently stored on this device.')
    ).toBeInTheDocument();
  });

  test('selecting Direct reveals the API key field', async () => {
    const Panel = await importPanel();
    renderWithProviders(<Panel />);
    await waitFor(() => expect(screen.queryByText('Loading…')).toBeNull());

    expect(screen.queryByLabelText('Composio API key')).toBeNull();
    fireEvent.click(screen.getByLabelText('Direct (bring your own API key)'));
    expect(screen.getByLabelText('Composio API key')).toBeInTheDocument();
  });

  test('saving Direct mode with a key calls setApiKey and masks the field', async () => {
    const Panel = await importPanel();
    renderWithProviders(<Panel />);
    await waitFor(() => expect(screen.queryByText('Loading…')).toBeNull());

    fireEvent.click(screen.getByLabelText('Direct (bring your own API key)'));
    const input = screen.getByLabelText('Composio API key') as HTMLInputElement;
    fireEvent.change(input, { target: { value: 'ck_secret_redacted' } });

    // First click on Save now opens the confirmation gate for the
    // Backend → Direct transition.
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(hoisted.setApiKey).not.toHaveBeenCalled();
    expect(
      screen.getByRole('button', { name: /I understand, switch to Direct/i })
    ).toBeInTheDocument();

    // Confirm the transition.
    fireEvent.click(screen.getByRole('button', { name: /I understand, switch to Direct/i }));

    await waitFor(() => {
      expect(screen.getByText('Settings saved')).toBeInTheDocument();
    });
    expect(hoisted.setApiKey).toHaveBeenCalledWith('ck_secret_redacted', true);
    // After save, the input is cleared so the secret isn't left in the DOM.
    expect(input.value).toBe('');
  });

  test('Backend → Direct shows confirmation dialog with warning copy', async () => {
    const Panel = await importPanel();
    renderWithProviders(<Panel />);
    await waitFor(() => expect(screen.queryByText('Loading…')).toBeNull());

    fireEvent.click(screen.getByLabelText('Direct (bring your own API key)'));
    fireEvent.change(screen.getByLabelText('Composio API key'), { target: { value: 'ck_test' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    // The warning dialog surfaces all three commitments the user is
    // signing up for — losing existing integrations, needing a personal
    // Composio account, and the trigger-webhook gap.
    const dialog = screen.getByRole('alertdialog');
    expect(dialog).toBeInTheDocument();
    expect(screen.getByText(/Switching to Direct mode/i)).toBeInTheDocument();
    expect(screen.getByText(/won.?t be visible/i)).toBeInTheDocument();
    // Scope the app.composio.dev lookup to the dialog because the same
    // hostname also appears in the API-key field's helper text above.
    expect(dialog.textContent ?? '').toMatch(/app\.composio\.dev/i);
    expect(screen.getByText(/triggers.*don.?t fire in Direct mode/i)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeInTheDocument();
  });

  test('Cancel button on confirmation dismisses without saving', async () => {
    const Panel = await importPanel();
    renderWithProviders(<Panel />);
    await waitFor(() => expect(screen.queryByText('Loading…')).toBeNull());

    fireEvent.click(screen.getByLabelText('Direct (bring your own API key)'));
    fireEvent.change(screen.getByLabelText('Composio API key'), { target: { value: 'ck_test' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(screen.getByRole('alertdialog')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    // Dialog dismissed; Save button visible again; no RPC call fired.
    expect(screen.queryByRole('alertdialog')).toBeNull();
    expect(screen.getByRole('button', { name: 'Save' })).toBeInTheDocument();
    expect(hoisted.setApiKey).not.toHaveBeenCalled();
  });

  test('Direct → Backend transition does NOT trigger the confirmation gate', async () => {
    // Recovery (Direct → Backend) is reversible — re-pasting the key
    // flips back instantly — so we skip the warning step to keep that
    // recovery cheap.
    hoisted.getMode.mockResolvedValue(directModeWithKey);
    const Panel = await importPanel();
    renderWithProviders(<Panel />);
    await waitFor(() => expect(screen.queryByText('Loading…')).toBeNull());

    fireEvent.click(screen.getByLabelText('Managed (OpenHuman handles it for you)'));
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    // No dialog appeared — clearApiKey was invoked straight through.
    expect(screen.queryByRole('alertdialog')).toBeNull();
    await waitFor(() => {
      expect(hoisted.clearApiKey).toHaveBeenCalled();
    });
  });

  test('Direct + replacement key (already in Direct mode) skips the gate', async () => {
    // The gate is only for Backend → Direct *transitions*. A user who
    // is already in Direct mode and is rotating their key should not
    // see the warning every time they save.
    hoisted.getMode.mockResolvedValue(directModeWithKey);
    const Panel = await importPanel();
    renderWithProviders(<Panel />);
    await waitFor(() => expect(screen.queryByText('Loading…')).toBeNull());

    fireEvent.change(screen.getByLabelText('Composio API key'), {
      target: { value: 'ck_new_key' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    expect(screen.queryByRole('alertdialog')).toBeNull();
    await waitFor(() => {
      expect(hoisted.setApiKey).toHaveBeenCalledWith('ck_new_key', true);
    });
  });

  test('the API key input is of type=password to avoid shoulder-surfing', async () => {
    hoisted.getMode.mockResolvedValue(directModeWithKey);
    const Panel = await importPanel();
    renderWithProviders(<Panel />);
    await waitFor(() => expect(screen.queryByText('Loading…')).toBeNull());

    const input = screen.getByLabelText('Composio API key') as HTMLInputElement;
    expect(input.type).toBe('password');
  });

  test('saving with Direct selected but no key shows an error (no RPC call)', async () => {
    const Panel = await importPanel();
    renderWithProviders(<Panel />);
    await waitFor(() => expect(screen.queryByText('Loading…')).toBeNull());

    fireEvent.click(screen.getByLabelText('Direct (bring your own API key)'));
    // Don't type anything.
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    expect(
      screen.getByText('Failed to save. Direct mode requires a non-empty API key.')
    ).toBeInTheDocument();
    expect(hoisted.setApiKey).not.toHaveBeenCalled();
  });

  test('switching to Backend mode calls clearApiKey and shows cleared status', async () => {
    hoisted.getMode.mockResolvedValue(directModeWithKey);
    const Panel = await importPanel();
    renderWithProviders(<Panel />);
    await waitFor(() => expect(screen.queryByText('Loading…')).toBeNull());

    fireEvent.click(screen.getByLabelText('Managed (OpenHuman handles it for you)'));
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    await waitFor(() => {
      expect(screen.getByText('Switched to Backend mode')).toBeInTheDocument();
    });
    expect(hoisted.clearApiKey).toHaveBeenCalledTimes(1);
  });

  test('shows error status when RPC throws', async () => {
    hoisted.setApiKey.mockRejectedValue(new Error('rpc error'));
    const Panel = await importPanel();
    renderWithProviders(<Panel />);
    await waitFor(() => expect(screen.queryByText('Loading…')).toBeNull());

    fireEvent.click(screen.getByLabelText('Direct (bring your own API key)'));
    fireEvent.change(screen.getByLabelText('Composio API key'), {
      target: { value: 'ck_secret_redacted' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    // Now in confirmation gate — confirm to actually call setApiKey.
    fireEvent.click(screen.getByRole('button', { name: /I understand, switch to Direct/i }));

    await waitFor(() => {
      expect(
        screen.getByText('Failed to save. Direct mode requires a non-empty API key.')
      ).toBeInTheDocument();
    });
  });

  test('panel still renders if getMode rejects (defaults to backend)', async () => {
    hoisted.getMode.mockRejectedValue(new Error('network down'));
    const Panel = await importPanel();
    renderWithProviders(<Panel />);
    await waitFor(() => expect(screen.queryByText('Loading…')).toBeNull());

    expect(screen.getByLabelText('Managed (OpenHuman handles it for you)')).toBeInTheDocument();
  });

  test('trigger-webhook gap is surfaced in the Direct mode description', async () => {
    const Panel = await importPanel();
    renderWithProviders(<Panel />);
    await waitFor(() => expect(screen.queryByText('Loading…')).toBeNull());

    // The "not yet routed" copy is the contract surface that flags the
    // direct-mode trigger gap to the user. If this assertion breaks,
    // update the catalog entry in about_app/catalog.rs in lockstep.
    expect(screen.getByText(/not yet routed/i)).toBeInTheDocument();
  });
});
