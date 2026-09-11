/**
 * Rendering tests for GatewaySection.
 *
 * `GatewaySection.test.tsx` covers `draftToGateway`, the pure half. This file
 * covers the half that talks to the shell: what is listed, what switching does,
 * how a failure is surfaced, and the form that produces a record.
 */
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { renderWithProviders } from '../../../../../test/test-utils';
import GatewaySection from '../GatewaySection';

const hoisted = vi.hoisted(() => ({
  listGateways: vi.fn(),
  activeGatewayId: vi.fn(),
  gatewayStatus: vi.fn(),
  activateGateway: vi.fn(),
  saveGateway: vi.fn(),
  deleteGateway: vi.fn(),
  clearCoreRpcUrlCache: vi.fn(),
  clearCoreRpcTokenCache: vi.fn(),
}));

vi.mock('../../../../../services/gatewayService', async importOriginal => {
  const actual = await importOriginal<typeof import('../../../../../services/gatewayService')>();
  return {
    ...actual,
    listGateways: hoisted.listGateways,
    activeGatewayId: hoisted.activeGatewayId,
    gatewayStatus: hoisted.gatewayStatus,
    activateGateway: hoisted.activateGateway,
    saveGateway: hoisted.saveGateway,
    deleteGateway: hoisted.deleteGateway,
  };
});

vi.mock('../../../../../services/coreRpcClient', () => ({
  clearCoreRpcUrlCache: hoisted.clearCoreRpcUrlCache,
  clearCoreRpcTokenCache: hoisted.clearCoreRpcTokenCache,
}));

const DESKTOP = { id: 'desktop', label: 'This computer', kind: 'desktop' };
const BUILDER = { id: 'builder', label: 'Build server', kind: 'ssh+docker' };

beforeEach(() => {
  Object.values(hoisted).forEach(fn => fn.mockReset());
  hoisted.listGateways.mockResolvedValue([DESKTOP, BUILDER]);
  hoisted.activeGatewayId.mockResolvedValue('desktop');
  hoisted.gatewayStatus.mockResolvedValue({
    state: 'connected',
    endpoint: 'http://127.0.0.1:7788',
  });
  hoisted.activateGateway.mockResolvedValue({ id: 'builder', rpcUrl: 'http://127.0.0.1:5/rpc' });
  hoisted.saveGateway.mockResolvedValue(undefined);
  hoisted.deleteGateway.mockResolvedValue(undefined);
  localStorage.clear();
});

describe('GatewaySection', () => {
  test('renders nothing when the build cannot reach gateways', () => {
    // A build without the shell feature has no commands to call. Rendering an
    // empty section would invite the user to configure something inert.
    const { container } = renderWithProviders(<GatewaySection available={false} />);

    expect(container).toBeEmptyDOMElement();
    expect(hoisted.listGateways).not.toHaveBeenCalled();
  });

  test('lists every gateway with the desktop one always present', async () => {
    renderWithProviders(<GatewaySection available />);

    expect(await screen.findByText('This computer')).toBeInTheDocument();
    expect(screen.getByText('Build server')).toBeInTheDocument();
  });

  test('the active gateway cannot be switched to and cannot be removed', async () => {
    // Removing the built-in would leave no guaranteed way back to a working
    // core, so it is not offered.
    renderWithProviders(<GatewaySection available />);

    await waitFor(() => expect(screen.getByTestId('gateway-use-desktop')).toBeDisabled());
    expect(screen.queryByTestId('gateway-remove-desktop')).not.toBeInTheDocument();
    expect(screen.getByTestId('gateway-remove-builder')).toBeInTheDocument();
  });

  test('switching activates and drops the cached url and bearer together', async () => {
    // The shell now answers from a different core; a stale cached bearer would
    // 401 every following call.
    renderWithProviders(<GatewaySection available />);

    fireEvent.click(await screen.findByTestId('gateway-use-builder'));

    await waitFor(() => expect(hoisted.activateGateway).toHaveBeenCalledWith('builder'));
    expect(hoisted.clearCoreRpcUrlCache).toHaveBeenCalled();
    expect(hoisted.clearCoreRpcTokenCache).toHaveBeenCalled();
    expect(localStorage.getItem('openhuman_core_mode')).toBe('gateway');
    expect(localStorage.getItem('openhuman_core_gateway_id')).toBe('builder');
  });

  test('switching back to this computer records local mode, not a gateway id', async () => {
    hoisted.activeGatewayId.mockResolvedValue('builder');
    renderWithProviders(<GatewaySection available />);

    fireEvent.click(await screen.findByTestId('gateway-use-desktop'));

    await waitFor(() => expect(localStorage.getItem('openhuman_core_mode')).toBe('local'));
  });

  test('a failed activation is surfaced rather than swallowed', async () => {
    hoisted.activateGateway.mockRejectedValue(new Error('could not reach the box'));
    renderWithProviders(<GatewaySection available />);

    fireEvent.click(await screen.findByTestId('gateway-use-builder'));

    expect(await screen.findByTestId('gateway-error')).toHaveTextContent('could not reach the box');
  });

  test('the status line reports the step while a gateway is being provisioned', async () => {
    // Provisioning takes tens of seconds; an untimed spinner would say nothing
    // about whether an image pull is stuck.
    hoisted.gatewayStatus.mockResolvedValue({ state: 'activating', step: 'creating the box' });
    renderWithProviders(<GatewaySection available />);

    expect(await screen.findByText(/creating the box/)).toBeInTheDocument();
  });

  test('a failed gateway shows why on its own row', async () => {
    hoisted.gatewayStatus.mockResolvedValue({ state: 'failed', reason: 'ssh: no route to host' });
    renderWithProviders(<GatewaySection available />);

    expect(await screen.findByText(/ssh: no route to host/)).toBeInTheDocument();
  });

  test('adding a gateway derives an id from the name and saves a record', async () => {
    renderWithProviders(<GatewaySection available />);

    fireEvent.click(await screen.findByTestId('gateway-add'));
    fireEvent.change(screen.getByLabelText(/Name/i), { target: { value: 'Build Server 2' } });
    fireEvent.click(screen.getByTestId('gateway-save'));

    await waitFor(() => expect(hoisted.saveGateway).toHaveBeenCalled());
    const saved = hoisted.saveGateway.mock.calls[0]?.[0];
    // Derived so the form asks one question instead of two.
    expect(saved.id).toBe('build-server-2');
    expect(saved.label).toBe('Build Server 2');
    expect(saved.spec).toMatchObject({ kind: 'box', reach: { kind: 'local' } });
  });

  test('choosing SSH reveals the destination field and carries it into the record', async () => {
    renderWithProviders(<GatewaySection available />);

    fireEvent.click(await screen.findByTestId('gateway-add'));
    fireEvent.change(screen.getByLabelText(/Name/i), { target: { value: 'remote' } });
    fireEvent.click(screen.getByLabelText(/over SSH/i));
    fireEvent.change(screen.getByLabelText(/SSH destination/i), {
      target: { value: 'builder@example.com' },
    });
    fireEvent.click(screen.getByTestId('gateway-save'));

    await waitFor(() => expect(hoisted.saveGateway).toHaveBeenCalled());
    expect(hoisted.saveGateway.mock.calls[0]?.[0].spec.reach).toMatchObject({
      kind: 'ssh',
      destination: 'builder@example.com',
    });
  });

  test('a form error is shown and nothing is saved', async () => {
    renderWithProviders(<GatewaySection available />);

    fireEvent.click(await screen.findByTestId('gateway-add'));
    // No name, so no id can be derived.
    fireEvent.click(screen.getByTestId('gateway-save'));

    expect(await screen.findByTestId('gateway-error')).toBeInTheDocument();
    expect(hoisted.saveGateway).not.toHaveBeenCalled();
  });

  test('cancelling the form clears it without saving', async () => {
    renderWithProviders(<GatewaySection available />);

    fireEvent.click(await screen.findByTestId('gateway-add'));
    expect(screen.getByTestId('gateway-form')).toBeInTheDocument();

    fireEvent.click(screen.getByText(/cancel/i));

    await waitFor(() => expect(screen.queryByTestId('gateway-form')).not.toBeInTheDocument());
    expect(hoisted.saveGateway).not.toHaveBeenCalled();
  });

  test('removing a gateway calls through and refreshes the list', async () => {
    renderWithProviders(<GatewaySection available />);

    fireEvent.click(await screen.findByTestId('gateway-remove-builder'));

    await waitFor(() => expect(hoisted.deleteGateway).toHaveBeenCalledWith('builder'));
    // Refreshed, so a deletion that raced another window still converges.
    expect(hoisted.listGateways.mock.calls.length).toBeGreaterThan(1);
  });

  test('a failed removal is surfaced', async () => {
    hoisted.deleteGateway.mockRejectedValue(new Error('read-only file system'));
    renderWithProviders(<GatewaySection available />);

    fireEvent.click(await screen.findByTestId('gateway-remove-builder'));

    expect(await screen.findByTestId('gateway-error')).toHaveTextContent('read-only file system');
  });

  test('an uncontained gateway asks for a binary path instead of an image', async () => {
    renderWithProviders(<GatewaySection available />);

    fireEvent.click(await screen.findByTestId('gateway-add'));
    fireEvent.click(screen.getByLabelText(/inside a container/i));

    expect(screen.getByLabelText(/openhuman-core/i)).toBeInTheDocument();
  });
});
