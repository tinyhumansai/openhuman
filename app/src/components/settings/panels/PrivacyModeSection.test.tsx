import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import PrivacyModeSection from './PrivacyModeSection';

const callCoreRpc = vi.fn();
vi.mock('../../../services/coreRpcClient', () => ({
  callCoreRpc: (arg: { method: string; params: unknown }) => callCoreRpc(arg),
}));

beforeEach(() => {
  vi.clearAllMocks();
  callCoreRpc.mockImplementation((arg: { method: string; params: { mode?: string } }) => {
    if (arg.method === 'openhuman.config_get_privacy_mode') {
      return Promise.resolve({ result: { mode: 'standard' } });
    }
    if (arg.method === 'openhuman.config_set_privacy_mode') {
      return Promise.resolve({ result: { mode: arg.params.mode } });
    }
    return Promise.reject(new Error(`unexpected method ${arg.method}`));
  });
});

describe('PrivacyModeSection', () => {
  it('renders the three privacy mode options', async () => {
    renderWithProviders(<PrivacyModeSection />);
    await waitFor(() =>
      expect(screen.getByTestId('privacy-mode-option-standard')).toBeInTheDocument()
    );
    expect(screen.getByTestId('privacy-mode-option-local_only')).toBeInTheDocument();
    expect(screen.getByTestId('privacy-mode-option-standard')).toBeInTheDocument();
    expect(screen.getByTestId('privacy-mode-option-sensitive')).toBeInTheDocument();
  });

  it('marks the loaded mode as selected', async () => {
    renderWithProviders(<PrivacyModeSection />);
    await waitFor(() =>
      expect(screen.getByTestId('privacy-mode-option-standard')).toHaveAttribute(
        'aria-checked',
        'true'
      )
    );
    expect(screen.getByTestId('privacy-mode-option-local_only')).toHaveAttribute(
      'aria-checked',
      'false'
    );
  });

  it('calls the set RPC with the chosen mode on selection', async () => {
    renderWithProviders(<PrivacyModeSection />);
    await waitFor(() =>
      expect(screen.getByTestId('privacy-mode-option-local_only')).toBeInTheDocument()
    );

    fireEvent.click(screen.getByTestId('privacy-mode-option-local_only'));

    await waitFor(() =>
      expect(callCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.config_set_privacy_mode',
        params: { mode: 'local_only' },
      })
    );

    // After the set resolves, the newly-chosen mode is selected.
    await waitFor(() =>
      expect(screen.getByTestId('privacy-mode-option-local_only')).toHaveAttribute(
        'aria-checked',
        'true'
      )
    );
  });

  // Finding 2 (#4437): saving a new mode must also push it into the Redux
  // privacy slice so the persistent status pill updates live, instead of
  // showing the stale mode until the next app reload.
  it('dispatches the new mode into the privacy slice on a successful save', async () => {
    const { store } = renderWithProviders(<PrivacyModeSection />);
    await waitFor(() =>
      expect(screen.getByTestId('privacy-mode-option-local_only')).toBeInTheDocument()
    );

    fireEvent.click(screen.getByTestId('privacy-mode-option-local_only'));

    await waitFor(() =>
      expect(
        (store.getState() as { privacy: { privacyMode: string | null } }).privacy.privacyMode
      ).toBe('local_only')
    );
  });

  it('does not re-issue the set RPC when the current mode is clicked', async () => {
    renderWithProviders(<PrivacyModeSection />);
    await waitFor(() =>
      expect(screen.getByTestId('privacy-mode-option-standard')).toHaveAttribute(
        'aria-checked',
        'true'
      )
    );

    fireEvent.click(screen.getByTestId('privacy-mode-option-standard'));

    // Only the initial get RPC should have fired — no set for an unchanged mode.
    expect(
      callCoreRpc.mock.calls.filter(c => c[0].method === 'openhuman.config_set_privacy_mode')
    ).toHaveLength(0);
  });
});
