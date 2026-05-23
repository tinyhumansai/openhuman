/**
 * TransportManager — selects and races transports given a ConnectionProfile.
 *
 * Desktop: defaults to LocalTransport; switches to CloudHttpTransport if
 * the profile specifies kind "cloud".
 *
 * iOS (kind "lan" | "tunnel"): races LAN (2 s timeout) vs Tunnel and uses
 * whichever responds first. Falls back to whichever is still healthy.
 */
import debug from 'debug';

import { CloudHttpTransport } from './CloudHttpTransport';
import type { CoreTransport } from './CoreTransport';
import { LanHttpTransport } from './LanHttpTransport';
import { LocalTransport } from './LocalTransport';
import type { ConnectionProfile } from './profileStore';
import { TunnelTransport } from './TunnelTransport';

const log = debug('transport:manager');
const logErr = debug('transport:manager:error');

const LAN_RACE_TIMEOUT_MS = 2_000;

// -- TransportManager --------------------------------------------------------

export class TransportManager {
  private active: CoreTransport | null = null;

  constructor(
    private readonly profile: ConnectionProfile,
    private readonly localRpcUrl: () => Promise<string>,
    private readonly localToken: () => Promise<string | null>,
    private readonly backendSocketUrl: string
  ) {}

  /**
   * Return the active transport, creating and health-checking it if needed.
   * For iOS profiles, races LAN vs Tunnel.
   */
  async getTransport(): Promise<CoreTransport> {
    if (this.active) {
      return this.active;
    }

    const transport = await this.selectTransport();
    this.active = transport;
    return transport;
  }

  /** Force re-selection (e.g. after a connection failure). */
  async reset(): Promise<void> {
    if (this.active) {
      await this.active.close().catch(() => {});
      this.active = null;
    }
  }

  async close(): Promise<void> {
    if (this.active) {
      await this.active.close().catch(() => {});
      this.active = null;
    }
  }

  // -- selection logic -------------------------------------------------------

  private async selectTransport(): Promise<CoreTransport> {
    const { kind } = this.profile;
    log('[transport:manager] selecting kind=%s id=%s', kind, this.profile.id);

    if (kind === 'local') {
      const t = new LocalTransport(this.localRpcUrl, this.localToken);
      log('[transport:manager] → LocalTransport');
      return t;
    }

    if (kind === 'cloud') {
      const { rpcUrl, sessionToken } = this.profile;
      if (!rpcUrl) {
        throw new Error('[transport:manager] cloud profile missing rpcUrl');
      }
      const t = new CloudHttpTransport(rpcUrl, sessionToken ?? null);
      log('[transport:manager] → CloudHttpTransport rpcUrl=%s', rpcUrl);
      return t;
    }

    if (kind === 'lan') {
      const { rpcUrl } = this.profile;
      if (!rpcUrl) {
        throw new Error('[transport:manager] lan profile missing rpcUrl');
      }
      const t = new LanHttpTransport(rpcUrl);
      log('[transport:manager] → LanHttpTransport rpcUrl=%s', rpcUrl);
      return t;
    }

    if (kind === 'tunnel') {
      return this.raceLanAndTunnel();
    }

    throw new Error(`[transport:manager] unknown profile kind: ${kind}`);
  }

  /**
   * Race LAN (with 2 s timeout) against Tunnel.
   * Whichever responds to `openhuman.ping` first wins.
   * If LAN wins but later fails, caller should call reset() to re-race.
   */
  private async raceLanAndTunnel(): Promise<CoreTransport> {
    const { rpcUrl, channelId, corePubkey, sessionToken, pairingToken } = this.profile;

    if (!channelId || !corePubkey) {
      throw new Error('[transport:manager] tunnel profile missing channelId or corePubkey');
    }

    const tunnelToken = sessionToken ?? pairingToken;
    if (!tunnelToken) {
      throw new Error('[transport:manager] tunnel profile missing sessionToken or pairingToken');
    }

    const tunnelTransport = new TunnelTransport(
      this.backendSocketUrl,
      channelId,
      corePubkey,
      tunnelToken
    );

    if (!rpcUrl) {
      // No LAN URL — tunnel only.
      log('[transport:manager] → TunnelTransport (no LAN URL)');
      return tunnelTransport;
    }

    const lanTransport = new LanHttpTransport(rpcUrl, LAN_RACE_TIMEOUT_MS);

    // Race: LAN vs Tunnel. First healthy transport wins.
    log('[transport:manager] racing LAN vs Tunnel channelId=%s', channelId);

    type Winner = { transport: CoreTransport; loser: CoreTransport };

    const lanRace = lanTransport
      .isHealthy()
      .then((ok): Winner | null =>
        ok ? { transport: lanTransport, loser: tunnelTransport } : null
      );

    const tunnelRace = tunnelTransport
      .isHealthy()
      .then((ok): Winner | null =>
        ok ? { transport: tunnelTransport, loser: lanTransport } : null
      );

    const winner = await Promise.race([lanRace, tunnelRace]);

    if (winner) {
      // Close the losing transport.
      void winner.loser.close().catch(() => {});
      log('[transport:manager] race winner: %s', winner.transport.kind);
      return winner.transport;
    }

    // Both failed in the race window — wait for whichever succeeds.
    logErr('[transport:manager] race: both transports unhealthy; waiting…');
    const result = await Promise.any([lanRace, tunnelRace]);
    if (result) {
      void result.loser.close().catch(() => {});
      log('[transport:manager] fallback winner: %s', result.transport.kind);
      return result.transport;
    }

    throw new Error('[transport:manager] all transports failed to connect');
  }
}

// -- convenience factory ------------------------------------------------------

/**
 * Build a TransportManager from a ConnectionProfile.
 * `localRpcUrl` / `localToken` are only needed for kind="local".
 */
export function createTransportManager(
  profile: ConnectionProfile,
  opts: {
    localRpcUrl?: () => Promise<string>;
    localToken?: () => Promise<string | null>;
    backendSocketUrl?: string;
  } = {}
): TransportManager {
  const noop = () => Promise.resolve(null);
  const noopStr = () => Promise.resolve('');
  return new TransportManager(
    profile,
    opts.localRpcUrl ?? noopStr,
    opts.localToken ?? noop,
    opts.backendSocketUrl ?? ''
  );
}
