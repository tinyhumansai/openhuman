/**
 * Tests for the staging-only "Trigger Sentry Test" row that
 * `DeveloperOptionsPanel` renders at the top when
 * `APP_ENVIRONMENT === 'staging'`. Covers visibility gating, the
 * idle/sending/sent/error state machine, and the failure path.
 */
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';

const hoisted = vi.hoisted(() => ({
  trigger: vi.fn(),
  appEnvironment: 'staging' as 'staging' | 'production' | 'development',
}));

vi.mock('../../../../services/analytics', () => ({ triggerSentryTestEvent: hoisted.trigger }));

vi.mock('../../../../utils/config', () => ({
  get APP_ENVIRONMENT() {
    return hoisted.appEnvironment;
  },
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateToSettings: vi.fn(),
    navigateBack: vi.fn(),
    breadcrumbs: [],
  }),
}));

async function importPanel() {
  const mod = await import('../DeveloperOptionsPanel');
  return mod.default;
}

describe('DeveloperOptionsPanel — Sentry test row', () => {
  beforeEach(() => {
    hoisted.trigger.mockReset();
    hoisted.appEnvironment = 'staging';
  });

  test('does not render the row in production builds', async () => {
    hoisted.appEnvironment = 'production';
    vi.resetModules();
    const Panel = await importPanel();
    renderWithProviders(<Panel />);
    expect(screen.queryByText(/Trigger Sentry Test/i)).toBeNull();
    expect(screen.queryByRole('button', { name: /Send test event/i })).toBeNull();
  });

  test('renders the staging row and fires the helper on click', async () => {
    hoisted.trigger.mockResolvedValue('event-id-xyz');
    vi.resetModules();
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    expect(screen.getByText(/Trigger Sentry Test/i)).toBeInTheDocument();
    const button = screen.getByRole('button', { name: /Send test event/i });

    fireEvent.click(button);

    await waitFor(() => {
      expect(hoisted.trigger).toHaveBeenCalledTimes(1);
    });

    await waitFor(() => {
      expect(screen.getByText(/Event sent\./i)).toBeInTheDocument();
    });
    expect(screen.getByText(/event-id-xyz/)).toBeInTheDocument();

    // Status updates must announce via an accessible live region — without
    // role="status" + aria-live, screen readers stay silent on click.
    const live = screen.getByRole('status');
    expect(live).toHaveAttribute('aria-live', 'polite');
    expect(live).toHaveTextContent(/Event sent.*event-id-xyz/);
  });

  test('shows "no id" branch when the helper resolves with undefined', async () => {
    hoisted.trigger.mockResolvedValue(undefined);
    vi.resetModules();
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    fireEvent.click(screen.getByRole('button', { name: /Send test event/i }));

    await waitFor(() => {
      expect(screen.getByText(/Event sent\./i)).toBeInTheDocument();
    });
    expect(screen.getByText(/no id/i)).toBeInTheDocument();
  });

  test('surfaces the error message when the helper throws', async () => {
    hoisted.trigger.mockRejectedValue(new Error('network broke'));
    vi.resetModules();
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    fireEvent.click(screen.getByRole('button', { name: /Send test event/i }));

    await waitFor(() => {
      expect(screen.getByText(/Failed: network broke/i)).toBeInTheDocument();
    });
  });
});
