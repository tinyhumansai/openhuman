import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import SubconsciousTriggersPanel from '../SubconsciousTriggersPanel';

// Mock the lowest level (callCoreRpc) so the REAL tauriCommands wrappers
// (subconsciousTriggersStatus / setSubconsciousTriggersEnabled) execute and
// get covered, while the HTTP/transport call is stubbed.
vi.mock('../../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));
// Identity translations keep assertions locale-independent (data-testid based).
vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

const STATUS_METHOD = 'openhuman.subconscious_triggers_status';
const SETTINGS_METHOD = 'openhuman.heartbeat_settings_set';

const DISABLED = {
  triggers_enabled: false,
  mode: 'off',
  max_promotions_per_hour: 30,
  orchestrator_running: false,
  queue_depth: null as number | null,
  orchestrator_thread_id: 'subconscious:orchestrator',
  user_thread_id: 'subconscious:user',
};
const ENABLED = {
  ...DISABLED,
  triggers_enabled: true,
  mode: 'event_driven',
  orchestrator_running: true,
  queue_depth: 0,
};

let current: typeof DISABLED;
let statusShouldThrow = false;

async function wireRpc() {
  const { callCoreRpc } = await import('../../../services/coreRpcClient');
  vi.mocked(callCoreRpc).mockImplementation(async ({ method }: { method: string }) => {
    if (method === STATUS_METHOD) {
      if (statusShouldThrow) throw new Error('boom');
      return { result: current, logs: [] } as never;
    }
    if (method === SETTINGS_METHOD) {
      return { result: { settings: {} }, logs: [] } as never;
    }
    return { result: {}, logs: [] } as never;
  });
}

describe('SubconsciousTriggersPanel', () => {
  beforeEach(async () => {
    vi.clearAllMocks();
    current = { ...DISABLED };
    statusShouldThrow = false;
    await wireRpc();
  });

  it('renders the disabled baseline with the activation hint', async () => {
    renderWithProviders(<SubconsciousTriggersPanel />);

    await waitFor(() => expect(screen.getByTestId('subconscious-triggers-status')).toBeTruthy());
    expect(screen.getByTestId('row-pipeline').textContent).toContain('common.disabled');
    expect(screen.getByTestId('row-orchestrator').textContent).toContain(
      'subconsciousTriggers.stopped'
    );
    expect(screen.getByTestId('row-queue').textContent).toContain('—');
    expect(screen.getByTestId('row-orchestrator-thread').textContent).toContain(
      'subconscious:orchestrator'
    );
    expect(screen.getByTestId('row-user-thread').textContent).toContain('subconscious:user');
    expect(screen.getByTestId('subconscious-triggers-disabled-hint')).toBeTruthy();
  });

  it('enabling toggles the pipeline on and re-fetches', async () => {
    const { callCoreRpc } = await import('../../../services/coreRpcClient');
    renderWithProviders(<SubconsciousTriggersPanel />);
    await waitFor(() => expect(screen.getByTestId('subconscious-triggers-toggle')).toBeTruthy());

    // After enabling, subsequent status reads return the enabled snapshot.
    current = { ...ENABLED };
    fireEvent.click(screen.getByTestId('subconscious-triggers-toggle'));

    expect(vi.mocked(callCoreRpc)).toHaveBeenCalledWith(
      expect.objectContaining({
        method: SETTINGS_METHOD,
        params: { triggers_enabled: true, subconscious_mode: 'event_driven' },
      })
    );
    await waitFor(() =>
      expect(screen.getByTestId('row-pipeline').textContent).toContain('common.enabled')
    );
    expect(screen.getByTestId('row-orchestrator').textContent).toContain(
      'subconsciousTriggers.running'
    );
    expect(screen.getByTestId('row-queue').textContent).not.toContain('—');
  });

  it('disabling sends triggers_enabled=false and mode=off', async () => {
    const { callCoreRpc } = await import('../../../services/coreRpcClient');
    current = { ...ENABLED };
    renderWithProviders(<SubconsciousTriggersPanel />);
    await waitFor(() =>
      expect(screen.getByTestId('row-pipeline').textContent).toContain('common.enabled')
    );

    fireEvent.click(screen.getByTestId('subconscious-triggers-toggle'));
    expect(vi.mocked(callCoreRpc)).toHaveBeenCalledWith(
      expect.objectContaining({
        method: SETTINGS_METHOD,
        params: { triggers_enabled: false, subconscious_mode: 'off' },
      })
    );
  });

  it('refresh re-fetches status', async () => {
    const { callCoreRpc } = await import('../../../services/coreRpcClient');
    renderWithProviders(<SubconsciousTriggersPanel />);
    await waitFor(() => expect(screen.getByTestId('subconscious-triggers-status')).toBeTruthy());

    const before = vi
      .mocked(callCoreRpc)
      .mock.calls.filter(([arg]) => (arg as { method: string }).method === STATUS_METHOD).length;
    fireEvent.click(screen.getByTestId('subconscious-triggers-refresh'));
    await waitFor(() => {
      const after = vi
        .mocked(callCoreRpc)
        .mock.calls.filter(([arg]) => (arg as { method: string }).method === STATUS_METHOD).length;
      expect(after).toBeGreaterThan(before);
    });
  });

  it('shows an error state when the status fetch fails', async () => {
    statusShouldThrow = true;
    renderWithProviders(<SubconsciousTriggersPanel />);
    await waitFor(() => expect(screen.getByTestId('subconscious-triggers-error')).toBeTruthy());
  });
});
