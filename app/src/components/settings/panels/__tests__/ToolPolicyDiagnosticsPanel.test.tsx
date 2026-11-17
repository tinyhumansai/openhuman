import { screen } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';

const hoisted = vi.hoisted(() => ({ callCoreRpc: vi.fn() }));

vi.mock('../../../../services/coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => hoisted.callCoreRpc(...args),
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));

describe('ToolPolicyDiagnosticsPanel', () => {
  beforeEach(() => hoisted.callCoreRpc.mockReset());

  test('renders active policy from security_policy_info', async () => {
    hoisted.callCoreRpc.mockResolvedValue({
      result: {
        autonomy: 'supervised',
        workspace_only: true,
        allowed_commands: ['ls'],
        max_actions_per_hour: 123,
        require_approval_for_medium_risk: true,
        block_high_risk_commands: true,
      },
      logs: ['policy loaded'],
    });

    const Panel = (await import('../ToolPolicyDiagnosticsPanel')).default;
    renderWithProviders(<Panel />);

    expect(await screen.findByTestId('policy-info-ready')).toBeInTheDocument();
    expect(screen.getByText('supervised')).toBeInTheDocument();
    expect(screen.getByText('ls')).toBeInTheDocument();
    expect(hoisted.callCoreRpc).toHaveBeenCalledWith(
      expect.objectContaining({ method: 'openhuman.security_policy_info' })
    );
  });
});
