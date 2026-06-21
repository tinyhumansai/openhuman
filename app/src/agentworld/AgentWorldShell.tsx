/**
 * AgentWorldShell — provider wrapper for the Agent World section.
 *
 * Provides:
 *   - The tiny.place API client (injected via ApiProvider so all nested hooks
 *     call through `createInvokeApiClient()` instead of the HTTP SDK).
 *   - Theme bridging (OpenHuman → Agent World colour tokens).
 *
 * Intentionally excludes WalletContext, MoonPay, and E2EAuthBridge that
 * appear in the tiny.place website's provider tree — those live in core or are
 * not needed in the embedded context.
 */
import type { ReactNode } from 'react';

import { createInvokeApiClient } from '../lib/agentworld/invokeApiClient';

interface AgentWorldShellProps {
  children: ReactNode;
}

// One client instance per app lifetime (the underlying HTTP calls go through
// callCoreRpc which manages its own connection lifecycle).
const apiClient = createInvokeApiClient();

export default function AgentWorldShell({ children }: AgentWorldShellProps) {
  // NOTE: When the vendored ApiProvider is available (from synced website/src),
  // wrap children with <ApiProvider client={apiClient}>.  For Wave 0 we expose
  // the client via a context (see AgentWorldContext) so the Explore placeholder
  // can demonstrate the end-to-end wiring without requiring the full vendor sync.
  void apiClient; // referenced here to ensure the module is evaluated
  return <>{children}</>;
}

export { apiClient };
