import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import ToolPolicyDiagnosticsPanel from './ToolPolicyDiagnosticsPanel';

const navigateBack = vi.fn();

vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack,
    breadcrumbs: [{ label: 'Settings' }, { label: 'Tool Policy' }],
  }),
}));

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

function diagnosticsResult(autonomyLevel = 'supervised') {
  return {
    total_tools: 12,
    enabled_tools: 9,
    mcp_stdio_tools: 3,
    json_rpc_tools: 9,
    possible_write_surfaces: ['memory.put'],
    policy_surfaces: ['tool_registry.diagnostics'],
    posture: {
      autonomy_level: autonomyLevel,
      workspace_only: true,
      max_actions_per_hour: 30,
      require_approval_for_medium_risk: true,
      block_high_risk_commands: true,
    },
    mcp_allowlists: { enabled: false, server_count: 0, enabled_server_count: 0, servers: [] },
    mcp_write_audit: { enabled: false, recent_rows: null, last_error: null },
    recent_denials: [],
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  callCoreRpc.mockResolvedValue(diagnosticsResult());
});

describe('<ToolPolicyDiagnosticsPanel />', () => {
  // Regression for TAURI-RUST-83E: the panel previously called the bare,
  // unregistered `tool_registry.diagnostics`, which the core dispatcher rejects
  // with "unknown method" (the registered name is the namespaced
  // `openhuman.tool_registry_diagnostics`). The mismatch broke the panel for
  // every user and flooded Sentry. Pin the exact registered method name.
  it('invokes the registered openhuman.tool_registry_diagnostics RPC method', async () => {
    render(<ToolPolicyDiagnosticsPanel />);

    await waitFor(() => {
      expect(callCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({ method: 'openhuman.tool_registry_diagnostics' })
      );
    });
  });

  it('renders the diagnostics posture once the RPC resolves', async () => {
    render(<ToolPolicyDiagnosticsPanel />);

    // The autonomy level only renders in the ready state, so finding it also
    // proves the panel left loading and did not fall into the error branch.
    expect(await screen.findByText('supervised')).toBeInTheDocument();
  });
});
