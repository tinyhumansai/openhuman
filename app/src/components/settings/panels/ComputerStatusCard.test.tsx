import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import ComputerStatusCard from './ComputerStatusCard';

const mocks = vi.hoisted(() => ({ rpc: vi.fn() }));
vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));
vi.mock('../../../services/coreRpcClient', () => ({ callCoreRpc: mocks.rpc }));

const base = {
  module: { id: 'tinycomputer', version: '0.7.0', state: 'available' },
  decision_model: 'jev',
  decision_route: 'hosted',
  planner_route: 'hosted',
};

beforeEach(() => {
  vi.clearAllMocks();
  mocks.rpc.mockResolvedValue(base);
});

describe('ComputerStatusCard', () => {
  it('reports the module and routes without loading it', async () => {
    renderWithProviders(<ComputerStatusCard />);
    await waitFor(() =>
      expect(mocks.rpc).toHaveBeenCalledWith({
        method: 'openhuman.modules_computer_status',
        params: { load: false },
      })
    );
    expect(await screen.findByText('computer.status.state.available')).toBeInTheDocument();
    expect(screen.getByText('v0.7.0')).toBeInTheDocument();
    expect(screen.getByTestId('computer-decision')).toHaveTextContent(
      'computer.models.jev · connections.browser.routeHosted'
    );
  });

  it('shows a module switched off by [components] as disabled', async () => {
    mocks.rpc.mockResolvedValueOnce({
      ...base,
      module: { ...base.module, state: 'disabled', detail: 'Switched off by [components]' },
    });
    renderWithProviders(<ComputerStatusCard />);
    expect(await screen.findByText('computer.status.state.disabled')).toBeInTheDocument();
  });

  it('checks the module and shows what it reports', async () => {
    renderWithProviders(<ComputerStatusCard />);
    await screen.findByText('computer.status.state.available');
    mocks.rpc.mockResolvedValueOnce({
      ...base,
      module: { ...base.module, state: 'ready' },
      decision_model: 'sage',
      decision_route: 'sage',
      capabilities: {
        contract_version: [2, 8],
        compatible: false,
        jev_configured: true,
        planner_configured: false,
        rescue_configured: true,
        surfaces: [
          { kind: 'desktop', available: false, reason: 'no permission' },
          { kind: 'browser', available: true },
        ],
      },
    });
    fireEvent.click(screen.getByText('computer.status.check'));
    await waitFor(() =>
      expect(mocks.rpc).toHaveBeenLastCalledWith({
        method: 'openhuman.modules_computer_status',
        params: { load: true },
      })
    );
    expect(await screen.findByText('computer.status.incompatible')).toBeInTheDocument();
    expect(screen.getByTestId('computer-decision')).toHaveTextContent(
      'computer.models.sage · computer.status.ownKey'
    );
    expect(screen.getByTestId('computer-capabilities')).toHaveTextContent(
      'computer.models.plannerModel: computer.status.missing'
    );
  });

  it('shows a module error and an RPC failure', async () => {
    mocks.rpc.mockResolvedValueOnce({
      ...base,
      decision_route: 'unavailable',
      planner_route: 'direct_openrouter',
      error: 'module faulted',
    });
    renderWithProviders(<ComputerStatusCard />);
    expect(await screen.findByText('module faulted')).toBeInTheDocument();
    expect(screen.getByTestId('computer-planner')).toHaveTextContent(
      'connections.browser.routeDirect'
    );
    mocks.rpc.mockRejectedValueOnce(new Error('core offline'));
    fireEvent.click(screen.getByText('computer.status.check'));
    expect(await screen.findByText('core offline')).toBeInTheDocument();
  });
});
