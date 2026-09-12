/**
 * CoreTransport interface — all core-RPC transports implement this.
 *
 * Implementations:
 *   LocalTransport   — local HTTP to the in-process core sidecar
 *   LanHttpTransport — HTTP to a LAN-accessible core URL
 *   TunnelTransport  — socket.io E2E-encrypted relay
 *   CloudHttpTransport — HTTP to a user-configured cloud core URL
 */

export type TransportKind = 'local' | 'lan-http' | 'tunnel' | 'cloud-http';

export interface CoreTransport {
  readonly kind: TransportKind;

  /** Make a JSON-RPC call and return the result. */
  /**
   * `timeoutMs`, when given, replaces the transport's own default deadline for
   * this one call. The RPC client forwards a caller's per-call budget here so a
   * request that legitimately runs for minutes (a memory source sync, a
   * coding-session import) is not cut off at the transport's default while the
   * core keeps working on it.
   */
  call<T>(
    method: string,
    params: unknown,
    opts?: { signal?: AbortSignal; timeoutMs?: number }
  ): Promise<T>;

  /** Stream a JSON-RPC method that produces sequential chunks. */
  stream<T>(method: string, params: unknown, opts?: { signal?: AbortSignal }): AsyncIterable<T>;

  /** Probe the transport with a ping. */
  isHealthy(): Promise<boolean>;

  /** Tear down the transport. */
  close(): Promise<void>;
}
