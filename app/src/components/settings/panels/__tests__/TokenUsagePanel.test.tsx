import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import * as tokenjuice from '../../../../utils/tauriCommands/tokenjuice';
import TokenUsagePanel from '../TokenUsagePanel';

vi.mock('../../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

vi.mock('../../../../utils/tauriCommands/tokenjuice', async () => {
  const actual = await vi.importActual<typeof import('../../../../utils/tauriCommands/tokenjuice')>(
    '../../../../utils/tauriCommands/tokenjuice'
  );
  return {
    ...actual,
    getTokenjuiceSettings: vi.fn(),
    getTokenjuiceSavings: vi.fn(),
    updateTokenjuiceSettings: vi.fn(),
    resetTokenjuiceSavings: vi.fn(),
  };
});

const mockGetSettings = vi.mocked(tokenjuice.getTokenjuiceSettings);
const mockGetSavings = vi.mocked(tokenjuice.getTokenjuiceSavings);
const mockUpdate = vi.mocked(tokenjuice.updateTokenjuiceSettings);

const stubSettings: tokenjuice.TokenjuiceSettings = {
  router_enabled: true,
  ccr_enabled: false,
  ccr_disk_enabled: true,
  max_cache_entries: 100,
  max_cache_bytes: 1024,
  ccr_ttl_secs: null,
  min_bytes_to_compress: 512,
  ccr_min_tokens: 1000,
  search_enabled: true,
  code_enabled: false,
  html_enabled: true,
  ml_compression_enabled: false,
  ml_model_id: 'model',
  ml_target_ratio: 0.5,
  ml_sidecar_idle_timeout_secs: 30,
  ml_max_input_chars: 4096,
  ml_device: 'cpu',
};

const stubSavings: tokenjuice.SavingsStats = {
  attributionModel: 'gpt-4',
  total: { events: 0, originalTokens: 0, compactedTokens: 0, tokensSaved: 0, costSavedUsd: 0 },
  byModel: {},
  byCompressor: {},
  cache: { entries: 0, bytes: 0 },
};

describe('TokenUsagePanel', () => {
  beforeEach(() => {
    mockGetSettings.mockReset();
    mockGetSavings.mockReset();
    mockUpdate.mockReset();
    mockGetSavings.mockResolvedValue(stubSavings);
  });

  describe('when settings load succeeds', () => {
    beforeEach(() => {
      mockGetSettings.mockResolvedValue(stubSettings);
    });

    it('enables all switches and the CCR min-tokens field once settings load', async () => {
      render(<TokenUsagePanel embedded />);

      // waitFor retries until switches transition from disabled (initial null state)
      // to enabled (after async settings load), proving the load actually settled.
      await waitFor(() => {
        const switches = screen.getAllByRole('switch');
        expect(switches).toHaveLength(7);
        for (const sw of switches) expect(sw).not.toBeDisabled();
      });
      expect(
        screen.getByRole('spinbutton', { name: 'settings.tokenUsage.ccrMinTokens' })
      ).not.toBeDisabled();
    });

    it('reflects the loaded toggle values', async () => {
      render(<TokenUsagePanel embedded />);

      // router_enabled=true, search_enabled=true, code_enabled=false in stubSettings.
      await waitFor(() =>
        expect(
          screen.getByRole('switch', { name: 'settings.tokenUsage.routerEnabled' })
        ).toBeChecked()
      );
      expect(screen.getByRole('switch', { name: 'settings.tokenUsage.search' })).toBeChecked();
      expect(screen.getByRole('switch', { name: 'settings.tokenUsage.code' })).not.toBeChecked();
    });

    it('calls patch when a switch is toggled', async () => {
      const user = userEvent.setup();
      mockUpdate.mockResolvedValue({ ...stubSettings, router_enabled: false });
      render(<TokenUsagePanel embedded />);

      const sw = await screen.findByRole('switch', { name: 'settings.tokenUsage.routerEnabled' });
      await user.click(sw);
      expect(mockUpdate).toHaveBeenCalledWith({ router_enabled: false });
    });

    it('keeps controls enabled and shows error when only savings fails', async () => {
      mockGetSavings.mockRejectedValue(new Error('savings rpc down'));
      render(<TokenUsagePanel embedded />);

      // Wait for the rendered savings error — proves the rejection settled and
      // React re-rendered (not just the initial settings === null disabled state).
      await screen.findByText('savings rpc down');

      // Controls remain interactive despite the savings failure.
      const switches = screen.getAllByRole('switch');
      expect(switches).toHaveLength(7);
      for (const sw of switches) expect(sw).not.toBeDisabled();
      expect(
        screen.getByRole('spinbutton', { name: 'settings.tokenUsage.ccrMinTokens' })
      ).not.toBeDisabled();
    });
  });

  describe('when settings load fails', () => {
    beforeEach(() => {
      mockGetSettings.mockRejectedValue(new Error('rpc down'));
    });

    it('disables all switches and the CCR min-tokens field while settings are unavailable', async () => {
      render(<TokenUsagePanel embedded />);

      // Wait for the rendered error — proves the settings rejection settled and
      // React re-rendered, ruling out the initial settings === null disabled state.
      await screen.findByText('rpc down');

      const switches = screen.getAllByRole('switch');
      expect(switches).toHaveLength(7);
      for (const sw of switches) expect(sw).toBeDisabled();
      expect(
        screen.getByRole('spinbutton', { name: 'settings.tokenUsage.ccrMinTokens' })
      ).toBeDisabled();
    });

    it('does not call patch when a disabled switch is clicked', async () => {
      const user = userEvent.setup();
      render(<TokenUsagePanel embedded />);

      // Wait for the rendered error before interacting.
      await screen.findByText('rpc down');

      const sw = screen.getByRole('switch', { name: 'settings.tokenUsage.routerEnabled' });
      expect(sw).toBeDisabled();
      await user.click(sw);
      expect(mockUpdate).not.toHaveBeenCalled();
    });
  });
});
