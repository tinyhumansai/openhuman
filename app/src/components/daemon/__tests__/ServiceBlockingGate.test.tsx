import { fireEvent, render, screen, waitFor } from '@testing-library/react';
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

  it('shows blocking screen when service is not installed', async () => {
    mockIsTauri.mockReturnValue(true);
    mockServiceStatus.mockResolvedValue({ result: { state: 'NotInstalled' }, logs: [] });
    mockAgentStatus.mockResolvedValue({ result: { running: false }, logs: [] });

    render(
      <ServiceBlockingGate>
        <div>App Content</div>
      </ServiceBlockingGate>
    );

    await waitFor(() => expect(screen.getByText('OpenHuman Service Required')).toBeInTheDocument());
    expect(screen.queryByText('App Content')).not.toBeInTheDocument();
    expect(screen.getByText('NotInstalled')).toBeInTheDocument();
  });

  it('runs install and start actions from blocker', async () => {
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

    await waitFor(() => expect(screen.getByText('OpenHuman Service Required')).toBeInTheDocument());

    fireEvent.click(screen.getByRole('button', { name: 'Install Service' }));
    await waitFor(() => expect(mockInstall).toHaveBeenCalled());
  });
});
