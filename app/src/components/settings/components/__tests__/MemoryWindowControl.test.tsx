/**
 * Tests for the user-facing memory-context window selector.
 *
 * Covers the wording the user sees (so the cost/continuity tradeoff is
 * surfaced explicitly), the persisted-preference roundtrip, and the
 * core RPC contract — the panel must call `update_memory_settings`
 * with the canonical lowercase preset label so the core stays the
 * source of truth for actual char budgets.
 */
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import MemoryWindowControl from '../MemoryWindowControl';

const hoisted = vi.hoisted(() => ({
  mockGetConfig: vi.fn(),
  mockUpdateMemorySettings: vi.fn(),
  mockIsTauri: vi.fn(() => true),
}));

vi.mock('../../../../utils/tauriCommands', () => ({
  isTauri: hoisted.mockIsTauri,
  openhumanGetConfig: hoisted.mockGetConfig,
  openhumanUpdateMemorySettings: hoisted.mockUpdateMemorySettings,
  MEMORY_CONTEXT_WINDOWS: ['minimal', 'balanced', 'extended', 'maximum'],
}));

beforeEach(() => {
  hoisted.mockGetConfig.mockReset();
  hoisted.mockUpdateMemorySettings.mockReset();
  hoisted.mockIsTauri.mockReturnValue(true);
});

const respondWithWindow = (memory_window: string | undefined) => {
  hoisted.mockGetConfig.mockResolvedValue({
    result: {
      config: { agent: memory_window === undefined ? {} : { memory_window } },
      workspace_dir: '/tmp/ws',
      config_path: '/tmp/cfg.toml',
    },
  });
};

describe('MemoryWindowControl', () => {
  it('explains the cost/continuity tradeoff in plain language', async () => {
    respondWithWindow('balanced');
    renderWithProviders(<MemoryWindowControl />);

    // Header copy makes the tradeoff explicit — increasing the window
    // costs more on every run.
    expect(screen.getByText(/larger windows feel more aware/i)).toHaveTextContent(
      /use more tokens/i
    );
    expect(screen.getByText(/cost more/i)).toBeInTheDocument();

    // All four presets are offered.
    for (const label of ['Minimal', 'Balanced', 'Extended', 'Maximum']) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }

    await waitFor(() => {
      expect(screen.getByTestId('memory-window-option-balanced')).toHaveAttribute(
        'aria-checked',
        'true'
      );
    });
  });

  it('reflects the persisted preference returned by the core', async () => {
    respondWithWindow('extended');
    renderWithProviders(<MemoryWindowControl />);

    await waitFor(() => {
      expect(screen.getByTestId('memory-window-option-extended')).toHaveAttribute(
        'aria-checked',
        'true'
      );
    });
    expect(screen.getByTestId('memory-window-hint')).toHaveTextContent(/more long-term memory/i);
  });

  it('falls back to balanced when the snapshot omits the field', async () => {
    respondWithWindow(undefined);
    renderWithProviders(<MemoryWindowControl />);

    await waitFor(() => {
      expect(screen.getByTestId('memory-window-option-balanced')).toHaveAttribute(
        'aria-checked',
        'true'
      );
    });
  });

  it('persists the chosen preset via update_memory_settings using the canonical label', async () => {
    respondWithWindow('balanced');
    hoisted.mockUpdateMemorySettings.mockResolvedValue({
      result: { config: { agent: { memory_window: 'maximum' } } },
    });
    const onSaved = vi.fn();
    renderWithProviders(<MemoryWindowControl onSaved={onSaved} />);

    await waitFor(() => expect(hoisted.mockGetConfig).toHaveBeenCalled());
    fireEvent.click(screen.getByTestId('memory-window-option-maximum'));

    await waitFor(() => {
      expect(hoisted.mockUpdateMemorySettings).toHaveBeenCalledWith({ memory_window: 'maximum' });
    });
    await waitFor(() => expect(onSaved).toHaveBeenCalledWith('maximum'));
    await waitFor(() => {
      expect(screen.getByTestId('memory-window-option-maximum')).toHaveAttribute(
        'aria-checked',
        'true'
      );
    });
  });

  it('surfaces save failures to the parent without rolling forward', async () => {
    respondWithWindow('balanced');
    hoisted.mockUpdateMemorySettings.mockRejectedValue(new Error('disk full'));
    const onError = vi.fn();
    renderWithProviders(<MemoryWindowControl onError={onError} />);

    await waitFor(() => expect(hoisted.mockGetConfig).toHaveBeenCalled());
    fireEvent.click(screen.getByTestId('memory-window-option-extended'));

    await waitFor(() => expect(onError).toHaveBeenCalledWith('disk full'));
    expect(screen.getByTestId('memory-window-option-balanced')).toHaveAttribute(
      'aria-checked',
      'true'
    );
  });
});
