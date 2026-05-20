/**
 * Shared Socket.IO factory for connections to the local OpenHuman core
 * (the in-process Rust server, addressed at `getCoreHttpBaseUrl()` or
 * the user's cloud-mode override).
 *
 * The core handshake validates the per-process bearer token, so every
 * caller has to read it via `getCoreRpcToken()` and pass it through
 * `io(url, { auth: { token } })`. Centralising the factory keeps the
 * handshake shape uniform across the three current call sites
 * (`socketService`, `useDictationHotkey`, `OverlayApp`) and gives each
 * site a single line to call.
 */
import { io, type ManagerOptions, type Socket, type SocketOptions } from 'socket.io-client';

export interface CoreSocketOptions {
  /**
   * Per-process core bearer (from `getCoreRpcToken()`). When `null` the
   * factory passes an empty string — the server will reject the
   * handshake, but tests that mock `io` need not bother priming the
   * token resolver.
   */
  coreToken: string | null;
  /**
   * Extra fields merged onto the `auth` payload. Today only the
   * authenticated user's session JWT goes here (under `session`) so a
   * future server-side handler can correlate the connection with the
   * logged-in user.
   */
  authExtras?: Record<string, unknown>;
  /**
   * Override of the underlying Socket.IO connect options. The default
   * shape matches what the previous in-line callers used.
   */
  overrides?: Partial<ManagerOptions & SocketOptions>;
}

const DEFAULT_OPTIONS: Partial<ManagerOptions & SocketOptions> = {
  path: '/socket.io/',
  transports: ['websocket', 'polling'],
  reconnection: true,
  reconnectionDelay: 2000,
  reconnectionAttempts: Infinity,
  forceNew: true,
};

export function createCoreSocket(baseUrl: string, opts: CoreSocketOptions): Socket {
  const auth = { token: opts.coreToken ?? '', ...(opts.authExtras ?? {}) };
  return io(baseUrl, { ...DEFAULT_OPTIONS, ...(opts.overrides ?? {}), auth });
}
