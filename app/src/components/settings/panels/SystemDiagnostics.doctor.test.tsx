import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import SystemDiagnostics from './SystemDiagnostics';

const callCoreRpc = vi.fn();
vi.mock('../../../services/coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => callCoreRpc(...args),
}));

const report = {
  schema_version: 2,
  profile: 'desktop-default',
  config_source: 'profile',
  tier: 'connect',
  checks_run: ['config/file-integrity', 'runtime/core-ready'],
  checks_skipped: [],
  findings: [
    {
      check_id: 'config/file-integrity',
      severity: 'warn',
      message: 'The config file was recovered from its backup.',
      fix_hint: 'Review the backup and restore a valid config file.',
      duration_ms: 4,
      affects: ['settings.configuration'],
    },
    {
      check_id: 'runtime/core-ready',
      severity: 'pass',
      message: 'The core is ready.',
      fix_hint: 'No action required.',
      duration_ms: 1,
      affects: [],
    },
  ],
  summary: { pass: 1, info: 0, warn: 1, error: 0, skipped: 0 },
  ok: true,
};

function renderDiagnostics() {
  return render(
    <MemoryRouter>
      <SystemDiagnostics />
    </MemoryRouter>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  callCoreRpc.mockResolvedValue({ result: report, logs: [] });
  vi.spyOn(window, 'confirm').mockReturnValue(true);
});

describe('<SystemDiagnostics /> doctor report', () => {
  it('renders v2 findings grouped by check category with the fix hint', async () => {
    renderDiagnostics();

    fireEvent.click(screen.getByRole('button', { name: 'Run doctor' }));

    await waitFor(() => {
      expect(callCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({ method: 'openhuman.doctor_report', params: { tier: 'connect' } })
      );
    });
    expect(await screen.findByText('config')).toBeInTheDocument();
    expect(screen.getByText('runtime')).toBeInTheDocument();
    expect(screen.getByText('The config file was recovered from its backup.')).toBeInTheDocument();
    expect(
      screen.getByText('Review the backup and restore a valid config file.')
    ).toBeInTheDocument();
    expect(screen.getByText('The core is ready.')).toBeInTheDocument();
  });

  it('requests the live tier only after the user confirms the token warning', async () => {
    vi.spyOn(window, 'confirm').mockReturnValue(false);
    renderDiagnostics();

    fireEvent.click(screen.getByRole('button', { name: 'Run live checks' }));
    expect(window.confirm).toHaveBeenCalled();
    expect(callCoreRpc).not.toHaveBeenCalled();

    vi.mocked(window.confirm).mockReturnValue(true);
    fireEvent.click(screen.getByRole('button', { name: 'Run live checks' }));

    await waitFor(() => {
      expect(callCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({ method: 'openhuman.doctor_report', params: { tier: 'live' } })
      );
    });
  });
});
