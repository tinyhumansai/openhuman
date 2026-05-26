import { screen, waitFor } from '@testing-library/react';
import { describe, expect, test, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';

const hoisted = vi.hoisted(() => ({ callCoreRpc: vi.fn() }));

vi.mock('../../../../services/coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => hoisted.callCoreRpc(...args),
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));

describe('ToolPolicyDiagnosticsPanel', () => {
  test('renders diagnostics from core RPC', async () => {
    hoisted.callCoreRpc.mockResolvedValue({
      total_tools: 10,
      enabled_tools: 10,
      mcp_stdio_tools: 3,
      json_rpc_tools: 7,
      possible_write_surfaces: ['tools.composio_execute'],
      policy_surfaces: ['security.policy_info'],
      posture: {
        autonomy_level: 'supervised',
        workspace_only: true,
        max_actions_per_hour: 123,
        require_approval_for_medium_risk: true,
        block_high_risk_commands: true,
      },
      mcp_allowlists: { enabled: true, server_count: 0, enabled_server_count: 0, servers: [] },
      mcp_write_audit: { enabled: true, recent_rows: 5, last_error: null },
      recent_denials: [],
    });

    const Panel = (await import('../ToolPolicyDiagnosticsPanel')).default;
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.getByText(/Policy posture/i)).toBeInTheDocument();
    });
    expect(screen.getByText('supervised')).toBeInTheDocument();
    expect(screen.getByText(/Total tools/i)).toBeInTheDocument();
    expect(screen.getAllByText('10').length).toBeGreaterThan(0);
    expect(screen.getByText(/Recent \(24h\): 5/i)).toBeInTheDocument();
    expect(hoisted.callCoreRpc).toHaveBeenCalled();
  });
});
