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

function policyResult(autonomy = 'supervised') {
  return {
    autonomy,
    workspace_only: true,
    allowed_commands: ['ls'],
    max_actions_per_hour: 30,
    require_approval_for_medium_risk: true,
    block_high_risk_commands: true,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  callCoreRpc.mockResolvedValue({ result: policyResult() });
});

describe('<ToolPolicyDiagnosticsPanel />', () => {
  it('invokes the registered security_policy_info RPC method', async () => {
    render(<ToolPolicyDiagnosticsPanel />);

    await waitFor(() => {
      expect(callCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({ method: 'openhuman.security_policy_info' })
      );
    });
  });

  it('renders the active security policy once the RPC resolves', async () => {
    callCoreRpc.mockResolvedValue({ result: policyResult() });
    render(<ToolPolicyDiagnosticsPanel />);

    expect(await screen.findByText('supervised')).toBeInTheDocument();
    expect(screen.getByText('ls')).toBeInTheDocument();
  });
});
