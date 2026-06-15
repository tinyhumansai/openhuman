import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import AgentActivityPanel from './AgentActivityPanel';

const navigateBack = vi.fn();

vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack,
    breadcrumbs: [{ label: 'Settings' }, { label: 'Agents' }],
  }),
}));

// Mock SettingsBackButton so the test does not depend on the route-aware
// visibility rules of the shared back button. We assert the panel wiring
// instead: that the back button renders and drives `navigateBack`, and that the
// level options drive the RPC.
vi.mock('../components/SettingsBackButton', () => ({
  default: ({ onBack }: { onBack?: () => void }) => (
    <button type="button" data-testid="settings-header-back" onClick={onBack}>
      back
    </button>
  ),
}));

const callCoreRpc = vi.fn();
vi.mock('../../../services/coreRpcClient', () => ({
  callCoreRpc: (arg: { method: string; params: unknown }) => callCoreRpc(arg),
}));

function settingsResult(level = 2) {
  return {
    result: {
      level,
      level_label: 'moderate',
      sync_interval_secs: 3600,
      heartbeat_enabled: true,
      subconscious_enabled: true,
      token_budget_per_cycle: null,
      estimated_monthly_cost_min_usd: 1,
      estimated_monthly_cost_max_usd: 5,
    },
  };
}

const costResult = { result: { month: '2026-06', total_cost_usd: 0, total_syncs: 0 } };

/** All buttons except the mocked SettingsHeader back button = the level options. */
function levelButtons() {
  return screen
    .getAllByRole('button')
    .filter(b => b.getAttribute('data-testid') !== 'settings-header-back');
}

beforeEach(() => {
  vi.clearAllMocks();
  callCoreRpc.mockImplementation((arg: { method: string }) => {
    switch (arg.method) {
      case 'openhuman.config_get_activity_level_settings':
        return Promise.resolve(settingsResult());
      case 'openhuman.memory_sources_monthly_cost_summary':
        return Promise.resolve(costResult);
      case 'openhuman.config_update_activity_level_settings':
        return Promise.resolve(settingsResult(4));
      default:
        return Promise.reject(new Error(`unexpected method ${arg.method}`));
    }
  });
});

describe('<AgentActivityPanel />', () => {
  it('renders the back button and the five level options once loaded', async () => {
    render(<AgentActivityPanel />);

    // The level options only render after the initial load resolves (the loading
    // state has none), so this also asserts the panel left the loading state.
    await waitFor(() => expect(levelButtons()).toHaveLength(5));
  });

  it('invokes the back handler from the back button', async () => {
    render(<AgentActivityPanel />);
    await screen.findByTestId('settings-header-back');

    fireEvent.click(screen.getByTestId('settings-header-back'));
    expect(navigateBack).toHaveBeenCalledTimes(1);
  });

  it('persists a new level selection via the update RPC', async () => {
    render(<AgentActivityPanel />);
    await waitFor(() => expect(levelButtons()).toHaveLength(5));

    // The last option is "Always-on" (level 4 -> api key "always_on").
    const options = levelButtons();
    fireEvent.click(options[options.length - 1]);

    await waitFor(() => {
      expect(callCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({
          method: 'openhuman.config_update_activity_level_settings',
          params: { level: 'always_on' },
        })
      );
    });
  });
});
