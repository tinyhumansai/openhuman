/**
 * gatewayService — the renderer's view of where its RPC goes.
 *
 * A *gateway* is one way of reaching an OpenHuman core: the one running inside
 * this app, a core somebody else is running at a URL, or one this app
 * provisions itself — in a Docker container, on a machine reached over SSH, or
 * a container on a machine over SSH.
 *
 * Two things are worth knowing before using this.
 *
 * **The renderer holds ids, not credentials.** A gateway's SSH destination,
 * identity path and bearer live in the Tauri shell's own store and never cross
 * into `localStorage`, where a renderer XSS could read them. `activate` answers
 * with an endpoint for the shell's own use; nothing here persists it.
 *
 * **Activating changes where every other RPC call goes.** The shell answers
 * `core_rpc_url` and `core_rpc_token` from the active gateway, so `coreRpcClient`
 * and every caller above it follow along with no change of their own. That is
 * the whole design: there is no per-gateway transport in the frontend.
 *
 * Shell side: `app/src-tauri/src/gateway/`.
 */
import { invoke } from '@tauri-apps/api/core';
import debug from 'debug';

import { isTauri } from '../utils/tauriCommands/common';

const log = debug('gateway');

/** How to reach the machine a core runs on. */
export type GatewayReach =
  | { kind: 'local' }
  | {
      kind: 'ssh';
      destination: string;
      port?: number;
      identity?: string;
      acceptNewHostKey?: boolean;
    };

/** What confines the core on that machine. */
export type GatewayConfinement =
  | { kind: 'passthrough'; binary: string; workspace?: string }
  | { kind: 'docker'; image: string };

/**
 * How to reach a core.
 *
 * `box` covers three user-facing cases — a container here, a machine over
 * there, a container over there — because reach and confinement are
 * independent choices rather than one enum. That mirrors the tinybox model the
 * shell drives, and is why "SSH plus Docker" needs no case of its own.
 */
export type GatewaySpec =
  | { kind: 'desktop' }
  | { kind: 'remote'; url: string; token?: string }
  | {
      kind: 'box';
      reach: GatewayReach;
      confinement: GatewayConfinement;
      env?: Record<string, string>;
    };

export interface Gateway {
  id: string;
  label: string;
  spec: GatewaySpec;
}

/**
 * The renderer-facing view of a configured gateway.
 *
 * Deliberately credential-free: the list UI only shows a label and a kind
 * badge. The remote bearer, SSH destination and identity path stay in the Tauri
 * shell store (see {@link GatewaySpec}) and never cross to the renderer.
 */
export interface GatewaySummary {
  id: string;
  label: string;
  kind: string;
}

/** What a gateway is doing right now. */
export type GatewayStatus =
  | { state: 'inactive' }
  | { state: 'activating'; step: string }
  | { state: 'connected'; endpoint: string }
  | { state: 'failed'; reason: string };

/** The id of the always-present gateway: the core inside this app. */
export const DESKTOP_GATEWAY_ID = 'desktop';

/**
 * Whether this build can reach gateways at all.
 *
 * The commands are absent — not stubbed — when the shell is built without its
 * `gateways` feature, and absent outside Tauri entirely. Callers check this
 * rather than catching an invoke rejection, so "this build cannot do that"
 * never gets rendered as "that gateway is broken".
 */
export function gatewaysAvailable(): boolean {
  // Guarded, not because `isTauri` is expected to throw, but because this is a
  // *capability probe* called during render: a probe that can throw turns
  // "gateways are unavailable here" into a blank settings panel. The repo's
  // Tauri rule asks for exactly this shape — `isTauri()` or a try/catch — and a
  // probe deserves both.
  try {
    return isTauri();
  } catch {
    return false;
  }
}

/** Every configured gateway, the desktop one first. */
export async function listGateways(): Promise<GatewaySummary[]> {
  if (!gatewaysAvailable()) return [];
  try {
    const gateways = await invoke<GatewaySummary[]>('gateway_list');
    log('listed %d gateway(s)', gateways.length);
    return gateways;
  } catch (err) {
    // A build without the feature, which is a fact about this app rather than
    // an error to surface. The picker falls back to the modes it can offer.
    log('gateway_list unavailable: %o', err);
    return [];
  }
}

/** Add or replace a gateway. Does not activate it. */
export async function saveGateway(gateway: Gateway): Promise<void> {
  if (!gatewaysAvailable()) {
    throw new Error('gateways are unavailable in this build');
  }
  await invoke('gateway_save', { gateway });
  log('saved gateway %s', gateway.id);
}

/** Forget a gateway. The running session is unaffected. */
export async function deleteGateway(id: string): Promise<void> {
  if (!gatewaysAvailable()) {
    throw new Error('gateways are unavailable in this build');
  }
  await invoke('gateway_delete', { id });
  log('deleted gateway %s', id);
}

/**
 * Make a gateway the one every RPC call goes to.
 *
 * Provisioning a box can take tens of seconds — a container start, the core's
 * own boot, possibly an image pull — so callers should show
 * {@link gatewayStatus} rather than blocking on this with a bare spinner.
 *
 * On failure the previously active gateway is still active: the shell tears the
 * old one down only after the new one answers.
 */
export async function activateGateway(id: string): Promise<void> {
  if (!gatewaysAvailable()) {
    throw new Error('gateways are unavailable in this build');
  }
  log('activating gateway %s', id);
  // The shell answers `core_rpc_url` / `core_rpc_token` from the active
  // gateway and never hands the bearer to the renderer — this call is an
  // acknowledgment that the switch happened.
  await invoke('gateway_activate', { id });
  log('gateway %s active', id);
}

/** Which gateway is active right now. */
export async function activeGatewayId(): Promise<string> {
  if (!gatewaysAvailable()) return DESKTOP_GATEWAY_ID;
  try {
    return await invoke<string>('gateway_active');
  } catch {
    return DESKTOP_GATEWAY_ID;
  }
}

/** What a gateway is doing right now. */
export async function gatewayStatus(id: string): Promise<GatewayStatus> {
  if (!gatewaysAvailable()) return { state: 'inactive' };
  try {
    return await invoke<GatewayStatus>('gateway_status', { id });
  } catch {
    return { state: 'inactive' };
  }
}

/**
 * A short label for what kind of gateway this is.
 *
 * Mirrors `GatewaySpec::kind` on the shell side. Kept here rather than sent
 * over so the picker can label an unsaved draft the user is still editing.
 */
export function gatewayKind(spec: GatewaySpec): string {
  if (spec.kind === 'desktop') return 'desktop';
  if (spec.kind === 'remote') return 'remote';
  const remote = spec.reach.kind === 'ssh';
  const contained = spec.confinement.kind === 'docker';
  if (remote && contained) return 'ssh+docker';
  if (remote) return 'ssh';
  if (contained) return 'docker';
  return 'local-process';
}
