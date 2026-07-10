/**
 * Tests for the Settings → Notification routing panel.
 *
 * Verifies the per-provider controls become interactive once settings load,
 * and that toggling the "route to orchestrator" checkbox persists the change
 * via setNotificationSettings.
 */
import { fireEvent, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import NotificationRoutingPanel from '../NotificationRoutingPanel';

const hoisted = vi.hoisted(() => ({
  fetchNotificationStats: vi.fn(),
  getNotificationSettings: vi.fn(),
  setNotificationSettings: vi.fn(),
}));

vi.mock('../../../../services/notificationService', () => ({
  fetchNotificationStats: hoisted.fetchNotificationStats,
  getNotificationSettings: hoisted.getNotificationSettings,
  setNotificationSettings: hoisted.setNotificationSettings,
}));

const providerSettings = { enabled: true, importance_threshold: 0.4, route_to_orchestrator: true };

describe('NotificationRoutingPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    hoisted.fetchNotificationStats.mockResolvedValue(null);
    hoisted.getNotificationSettings.mockResolvedValue(providerSettings);
    hoisted.setNotificationSettings.mockResolvedValue(undefined);
  });

  it('persists a route-to-orchestrator toggle once provider settings load', async () => {
    renderWithProviders(<NotificationRoutingPanel embedded />);

    // The gmail orchestrator checkbox becomes enabled after settings load.
    const orchestratorToggle = await waitFor(() => {
      const el = document.getElementById('notification-orchestrator-gmail') as HTMLInputElement;
      expect(el).not.toBeNull();
      expect(el).not.toBeDisabled();
      return el;
    });

    // It starts checked (route_to_orchestrator: true); toggle it off.
    expect(orchestratorToggle.checked).toBe(true);
    fireEvent.click(orchestratorToggle);

    await waitFor(() =>
      expect(hoisted.setNotificationSettings).toHaveBeenCalledWith(
        expect.objectContaining({ provider: 'gmail', route_to_orchestrator: false })
      )
    );
  });
});
