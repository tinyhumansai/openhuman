import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { ClaudeCodeStatusCard } from '../ClaudeCodeStatusCard';

const probe = vi.fn();
const authProbe = vi.fn();
const loginLaunch = vi.fn();

vi.mock('../../../../../utils/tauriCommands/config', () => ({
  openhumanClaudeCodeStatus: () => probe(),
  openhumanClaudeCodeAuthStatus: () => authProbe(),
  openhumanClaudeCodeLoginLaunch: () => loginLaunch(),
}));

describe('ClaudeCodeStatusCard', () => {
  beforeEach(() => {
    probe.mockReset();
    authProbe.mockReset();
    loginLaunch.mockReset();
    loginLaunch.mockResolvedValue('cmd');
    // Default auth response — individual tests override as needed.
    authProbe.mockResolvedValue({ result: { source: 'none', last_checked: 0 } });
  });

  it('renders the installed version + path when CC is OK', async () => {
    probe.mockResolvedValueOnce({
      result: { status: 'ok', version: '2.0.4', path: '/usr/local/bin/claude' },
    });
    render(<ClaudeCodeStatusCard />);
    await waitFor(() => {
      expect(screen.getByText(/Installed \(2\.0\.4\)/)).toBeInTheDocument();
    });
    expect(screen.getByText('/usr/local/bin/claude')).toBeInTheDocument();
  });

  it('shows the install hint when the binary is missing', async () => {
    probe.mockResolvedValueOnce({ result: { status: 'not_installed' } });
    render(<ClaudeCodeStatusCard />);
    await waitFor(() => {
      expect(screen.getByText(/Claude Code CLI is not installed/i)).toBeInTheDocument();
    });
  });

  it('shows the outdated state with min_required', async () => {
    probe.mockResolvedValueOnce({
      result: {
        status: 'outdated',
        version: '1.9.0',
        min_required: '2.0.0',
        path: '/usr/local/bin/claude',
      },
    });
    render(<ClaudeCodeStatusCard />);
    await waitFor(() => {
      expect(screen.getByText(/Outdated — found 1\.9\.0, need ≥ 2\.0\.0/)).toBeInTheDocument();
    });
  });

  it('surfaces a probe error', async () => {
    probe.mockRejectedValueOnce(new Error('boom'));
    render(<ClaudeCodeStatusCard />);
    await waitFor(() => {
      expect(screen.getByText(/Failed to probe: boom/)).toBeInTheDocument();
    });
  });

  it('re-probes when Refresh is clicked', async () => {
    probe
      .mockResolvedValueOnce({ result: { status: 'not_installed' } })
      .mockResolvedValueOnce({ result: { status: 'ok', version: '2.0.4', path: '/x/y/claude' } });
    const user = userEvent.setup();
    render(<ClaudeCodeStatusCard />);
    await waitFor(() => {
      expect(screen.getByText(/Claude Code CLI is not installed/i)).toBeInTheDocument();
    });
    await user.click(screen.getByRole('button', { name: /Probe/i }));
    await waitFor(() => {
      expect(screen.getByText(/Installed \(2\.0\.4\)/)).toBeInTheDocument();
    });
    expect(probe).toHaveBeenCalledTimes(2);
  });

  it('shows subscription auth with account email', async () => {
    probe.mockResolvedValueOnce({
      result: { status: 'ok', version: '2.0.4', path: '/usr/local/bin/claude' },
    });
    authProbe.mockReset();
    authProbe.mockResolvedValueOnce({
      result: {
        source: 'subscription',
        account_email: 'jamie@example.com',
        expires_at: '2026-06-01T00:00:00Z',
        last_checked: 1700000000,
      },
    });
    render(<ClaudeCodeStatusCard />);
    await waitFor(() => {
      expect(screen.getByText(/jamie@example\.com/)).toBeInTheDocument();
    });
    expect(screen.getByText(/claude logout/)).toBeInTheDocument();
  });

  it('shows API key env auth state', async () => {
    probe.mockResolvedValueOnce({ result: { status: 'not_installed' } });
    authProbe.mockReset();
    authProbe.mockResolvedValueOnce({ result: { source: 'api_key_env', last_checked: 0 } });
    render(<ClaudeCodeStatusCard />);
    await waitFor(() => {
      expect(screen.getByText(/detected in environment/i)).toBeInTheDocument();
    });
  });

  it('shows not-signed-in with claude login hint', async () => {
    probe.mockResolvedValueOnce({ result: { status: 'not_installed' } });
    render(<ClaudeCodeStatusCard />);
    await waitFor(() => {
      expect(screen.getByText(/Not signed in\./)).toBeInTheDocument();
    });
    expect(screen.getByText(/claude login/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Sign in with Claude/i })).toBeInTheDocument();
  });

  it('Sign in with Claude button launches login terminal', async () => {
    probe.mockResolvedValueOnce({ result: { status: 'ok', version: '2.0.4', path: '/x/y' } });
    const user = userEvent.setup();
    render(<ClaudeCodeStatusCard />);
    const btn = await screen.findByRole('button', { name: /Sign in with Claude/i });
    await user.click(btn);
    expect(loginLaunch).toHaveBeenCalledTimes(1);
  });

  it('Recheck triggers a second auth probe without re-running version probe', async () => {
    probe.mockResolvedValueOnce({
      result: { status: 'ok', version: '2.0.4', path: '/x/y/claude' },
    });
    authProbe.mockReset();
    authProbe
      .mockResolvedValueOnce({ result: { source: 'none', last_checked: 0 } })
      .mockResolvedValueOnce({
        result: {
          source: 'subscription',
          account_email: 'user@example.com',
          expires_at: null,
          last_checked: 1,
        },
      });
    const user = userEvent.setup();
    render(<ClaudeCodeStatusCard />);
    await waitFor(() => {
      expect(screen.getByText(/Not signed in\./)).toBeInTheDocument();
    });
    await user.click(screen.getByRole('button', { name: /Recheck/i }));
    await waitFor(() => {
      expect(screen.getByText(/user@example\.com/)).toBeInTheDocument();
    });
    expect(probe).toHaveBeenCalledTimes(1);
    expect(authProbe).toHaveBeenCalledTimes(2);
  });
});
