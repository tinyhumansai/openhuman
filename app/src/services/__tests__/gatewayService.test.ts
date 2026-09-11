/**
 * Tests for the gateway service.
 *
 * The interesting behaviour here is not the invoke plumbing but the two
 * decisions this module makes on the renderer's behalf: what a build without
 * the shell feature should look like (absent, not broken), and how a
 * reach/confinement pair is labelled to a user.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  activateGateway,
  activeGatewayId,
  deleteGateway,
  DESKTOP_GATEWAY_ID,
  gatewayKind,
  type GatewaySpec,
  gatewayStatus,
  listGateways,
  saveGateway,
} from '../gatewayService';

const invoke = vi.fn();
const isTauri = vi.fn(() => true);

vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock('../../utils/tauriCommands/common', () => ({ isTauri: () => isTauri() }));

beforeEach(() => {
  invoke.mockReset();
  isTauri.mockReturnValue(true);
});

describe('gatewayKind', () => {
  it('names the pairing rather than treating it as a third kind', () => {
    // Reach and confinement are independent choices, so "a container on the
    // build server" is those two answers, not a separate option.
    const sshDocker: GatewaySpec = {
      kind: 'box',
      reach: { kind: 'ssh', destination: 'builder@example.com' },
      confinement: { kind: 'docker', image: 'openhuman-core:latest' },
    };
    const localDocker: GatewaySpec = {
      kind: 'box',
      reach: { kind: 'local' },
      confinement: { kind: 'docker', image: 'openhuman-core:latest' },
    };
    const sshBare: GatewaySpec = {
      kind: 'box',
      reach: { kind: 'ssh', destination: 'builder@example.com' },
      confinement: { kind: 'passthrough', binary: '/usr/local/bin/openhuman-core' },
    };

    expect(gatewayKind(sshDocker)).toBe('ssh+docker');
    expect(gatewayKind(localDocker)).toBe('docker');
    expect(gatewayKind(sshBare)).toBe('ssh');
  });

  it('labels the two non-provisioned kinds', () => {
    expect(gatewayKind({ kind: 'desktop' })).toBe('desktop');
    expect(gatewayKind({ kind: 'remote', url: 'https://core.example.com/rpc' })).toBe('remote');
  });
});

describe('outside Tauri', () => {
  it('lists nothing rather than invoking a command that cannot exist', () => {
    isTauri.mockReturnValue(false);

    return listGateways().then(gateways => {
      expect(gateways).toEqual([]);
      expect(invoke).not.toHaveBeenCalled();
    });
  });

  it('reports the desktop gateway as active', async () => {
    isTauri.mockReturnValue(false);

    await expect(activeGatewayId()).resolves.toBe(DESKTOP_GATEWAY_ID);
  });
});

describe('a build without the shell feature', () => {
  it('treats an absent command as "this build cannot", not "that gateway is broken"', async () => {
    // The commands are absent rather than stubbed when the shell is built
    // without `gateways`, so an invoke rejection here is a fact about the
    // build. Surfacing it as an error would make the picker show a failure
    // for something the user never configured.
    invoke.mockRejectedValue(new Error('Command gateway_list not found'));

    await expect(listGateways()).resolves.toEqual([]);
    await expect(activeGatewayId()).resolves.toBe(DESKTOP_GATEWAY_ID);
    await expect(gatewayStatus('anything')).resolves.toEqual({ state: 'inactive' });
  });
});

describe('listGateways', () => {
  it('passes the credential-free summary through unchanged', async () => {
    const gateways = [
      { id: 'desktop', label: 'This computer', kind: 'desktop' },
      { id: 'builder', label: 'Build server', kind: 'ssh+docker' },
    ];
    invoke.mockResolvedValue(gateways);

    await expect(listGateways()).resolves.toEqual(gateways);
    expect(invoke).toHaveBeenCalledWith('gateway_list');
  });
});

describe('mutating and activating', () => {
  it('saves a gateway without activating it', async () => {
    // Editing an SSH destination should not tear the session down and
    // re-provision on every keystroke that lands in a save.
    invoke.mockResolvedValue(undefined);
    const gateway = {
      id: 'builder',
      label: 'Build server',
      spec: {
        kind: 'box' as const,
        reach: { kind: 'local' as const },
        confinement: { kind: 'docker' as const, image: 'openhuman-core:local' },
      },
    };

    await saveGateway(gateway);

    expect(invoke).toHaveBeenCalledWith('gateway_save', { gateway });
    expect(invoke).not.toHaveBeenCalledWith('gateway_activate', expect.anything());
  });

  it('deletes a gateway by id', async () => {
    invoke.mockResolvedValue(undefined);

    await deleteGateway('builder');

    expect(invoke).toHaveBeenCalledWith('gateway_delete', { id: 'builder' });
  });

  it('acknowledges an activation without exposing credentials to the renderer', async () => {
    invoke.mockResolvedValue(undefined);

    await expect(activateGateway('builder')).resolves.toBeUndefined();
    expect(invoke).toHaveBeenCalledWith('gateway_activate', { id: 'builder' });
  });

  it('lets an activation failure reach the caller', async () => {
    // Unlike the read paths, this one must not be swallowed: the user asked
    // for a switch and needs to know it did not happen.
    invoke.mockRejectedValue(new Error('could not reach the box'));

    await expect(activateGateway('builder')).rejects.toThrow('could not reach the box');
  });

  it('reports the active gateway and its status', async () => {
    invoke.mockResolvedValueOnce('builder');
    await expect(activeGatewayId()).resolves.toBe('builder');

    invoke.mockResolvedValueOnce({ state: 'connected', endpoint: 'http://127.0.0.1:54321' });
    await expect(gatewayStatus('builder')).resolves.toEqual({
      state: 'connected',
      endpoint: 'http://127.0.0.1:54321',
    });
  });
});
