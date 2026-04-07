import { render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, type Mock } from 'vitest';

import * as tauriCommands from '../../../utils/tauriCommands';
import ServiceBlockingGate from '../ServiceBlockingGate';

describe('ServiceBlockingGate', () => {
  const mockIsTauri = tauriCommands.isTauri as Mock;
  const mockServiceStatus = tauriCommands.openhumanServiceStatus as Mock;
  const mockAgentStatus = tauriCommands.openhumanAgentServerStatus as Mock;
  const mockInstall = tauriCommands.openhumanServiceInstall as Mock;
  const mockStart = tauriCommands.openhumanServiceStart as Mock;

  it('renders children directly outside Tauri', async () => {
    mockIsTauri.mockReturnValue(false);

    render(
      <ServiceBlockingGate>
        <div>App Content</div>
      </ServiceBlockingGate>
    );

    await waitFor(() => expect(screen.getByText('App Content')).toBeInTheDocument());
  });

  it('renders children even when service is not installed', async () => {
    mockIsTauri.mockReturnValue(true);
    mockServiceStatus.mockResolvedValue({ result: { state: 'NotInstalled' }, logs: [] });
    mockAgentStatus.mockResolvedValue({ result: { running: false }, logs: [] });

    render(
      <ServiceBlockingGate>
        <div>App Content</div>
      </ServiceBlockingGate>
    );

    await waitFor(() => expect(screen.getByText('App Content')).toBeInTheDocument());
    expect(screen.queryByText('OpenHuman Service Required')).not.toBeInTheDocument();
  });

  it('does not expose forced install actions from the app shell', async () => {
    mockIsTauri.mockReturnValue(true);
    mockServiceStatus.mockResolvedValue({ result: { state: 'NotInstalled' }, logs: [] });
    mockAgentStatus.mockResolvedValue({ result: { running: false }, logs: [] });
    mockInstall.mockResolvedValue({ result: { state: 'Stopped' }, logs: [] });
    mockStart.mockResolvedValue({ result: { state: 'Running' }, logs: [] });

    render(
      <ServiceBlockingGate>
        <div>App Content</div>
      </ServiceBlockingGate>
    );

    await waitFor(() => expect(screen.getByText('App Content')).toBeInTheDocument());
    expect(screen.queryByRole('button', { name: 'Install Service' })).not.toBeInTheDocument();
    expect(mockInstall).not.toHaveBeenCalled();
    expect(mockStart).not.toHaveBeenCalled();
  });

  it('renders children when service is running even if agent is not running', async () => {
    mockIsTauri.mockReturnValue(true);
    mockServiceStatus.mockResolvedValue({ result: { state: 'Running' }, logs: [] });
    mockAgentStatus.mockResolvedValue({ result: { running: false }, logs: [] });

    render(
      <ServiceBlockingGate>
        <div>App Content</div>
      </ServiceBlockingGate>
    );

    await waitFor(() => expect(screen.getByText('App Content')).toBeInTheDocument());
    expect(screen.queryByText('OpenHuman Service Required')).not.toBeInTheDocument();
  });

  it('renders children when service is running and agent probe fails', async () => {
    mockIsTauri.mockReturnValue(true);
    mockServiceStatus.mockResolvedValue({ result: { state: 'Running' }, logs: [] });
    mockAgentStatus.mockRejectedValue(new Error('agent status unavailable'));

    render(
      <ServiceBlockingGate>
        <div>App Content</div>
      </ServiceBlockingGate>
    );

    await waitFor(() => expect(screen.getByText('App Content')).toBeInTheDocument());
    expect(screen.queryByText('OpenHuman Service Required')).not.toBeInTheDocument();
  });

  it('renders children when service is stopped but agent server is running (soft pass)', async () => {
    mockIsTauri.mockReturnValue(true);
    mockServiceStatus.mockResolvedValue({ result: { state: 'Stopped' }, logs: [] });
    mockAgentStatus.mockResolvedValue({ result: { running: true }, logs: [] });

    render(
      <ServiceBlockingGate>
        <div>App Content</div>
      </ServiceBlockingGate>
    );

    await waitFor(() => expect(screen.getByText('App Content')).toBeInTheDocument());
    expect(screen.queryByText('OpenHuman Service Required')).not.toBeInTheDocument();
  });

  it('renders children when service probe fails but agent server is running (soft pass)', async () => {
    mockIsTauri.mockReturnValue(true);
    mockServiceStatus.mockRejectedValue(new Error('service status unavailable'));
    mockAgentStatus.mockResolvedValue({ result: { running: true }, logs: [] });

    render(
      <ServiceBlockingGate>
        <div>App Content</div>
      </ServiceBlockingGate>
    );

    await waitFor(() => expect(screen.getByText('App Content')).toBeInTheDocument());
    expect(screen.queryByText('OpenHuman Service Required')).not.toBeInTheDocument();
  });
});
