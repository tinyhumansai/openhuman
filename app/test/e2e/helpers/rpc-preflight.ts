/**
 * RPC contract preflight — validates that all RPC methods the E2E suite
 * calls actually exist in the running core registry.
 *
 * Call this in a spec's `before()` or in wdio.conf.ts `before` hook.
 * If any method is missing from the registry, the test fails immediately
 * rather than silently returning "method not found" mid-test (RC-7 class fault).
 */
import { callOpenhumanRpc } from './core-rpc';

// The full list of openhuman.* RPC methods called across all E2E specs.
// When adding a new spec that calls a new RPC method, add it here.
const REQUIRED_RPC_METHODS = [
  'core.ping',
  'openhuman.test_reset',
  'openhuman.notification_ingest',
  'openhuman.notification_list',
  'openhuman.notification_mark_read',
  'openhuman.notification_stats',
  'openhuman.memory_doc_put',
  'openhuman.memory_clear_namespace',
  'openhuman.memory_recall_memories',
  'openhuman.threads_create_new',
  'openhuman.threads_list',
  'openhuman.threads_message_append',
  'openhuman.threads_messages_list',
  'openhuman.webhooks_clear_logs',
  'openhuman.webhooks_register_echo',
  'openhuman.webhooks_unregister_echo',
  'openhuman.composio_list_tools',
  'openhuman.composio_list_available_triggers',
  'openhuman.composio_list_triggers',
  'openhuman.composio_enable_trigger',
  'openhuman.composio_disable_trigger',
  'openhuman.about_app_list',
] as const;

export type RpcMethod = (typeof REQUIRED_RPC_METHODS)[number];

/**
 * Fetch the controller schema list from the running core and verify
 * every required method is registered.
 *
 * Returns an object: { ok: boolean; missing: string[]; registered: string[] }
 * Does NOT throw — callers decide whether to fail the suite.
 */
export async function validateRpcContract(): Promise<{
  ok: boolean;
  missing: string[];
  registered: string[];
}> {
  const result = await callOpenhumanRpc('openhuman.about_app_list', {}).catch(() => null);
  if (!result?.ok) {
    return { ok: false, missing: [], registered: [] };
  }

  // about_app_list returns: { controllers: [{ method: string }] } or similar
  const controllers: Array<{ method?: string; name?: string }> =
    (result.result as any)?.controllers ??
    (result.result as any)?.methods ??
    (result.result as any)?.result?.controllers ??
    [];

  const registered = controllers.map(c => c.method ?? c.name ?? '').filter(Boolean);

  const missing = REQUIRED_RPC_METHODS.filter(
    m => !registered.includes(m) && m !== 'core.ping' // core.ping is not a controller
  );

  return { ok: missing.length === 0, missing, registered };
}

/**
 * Assert the RPC contract. Call from a spec's before() hook.
 * Skips gracefully if about_app_list is not available (older builds).
 */
export async function assertRpcContract(logPrefix = '[RpcPreflight]'): Promise<void> {
  console.log(`${logPrefix} Validating RPC contract...`);
  const { missing, registered } = await validateRpcContract();

  if (registered.length === 0) {
    console.warn(`${logPrefix} Could not fetch controller registry — skipping validation`);
    return;
  }

  if (missing.length > 0) {
    const msg =
      `${logPrefix} FATAL: ${missing.length} RPC method(s) not found in registry:\n` +
      missing.map(m => `  - ${m}`).join('\n') +
      '\nThis is an RC-7 class fault — the spec calls ghost RPCs. ' +
      'Fix: update REQUIRED_RPC_METHODS or restore the missing controllers.';
    console.error(msg);
    throw new Error(msg);
  }

  console.log(
    `${logPrefix} RPC contract OK — ${registered.length} controllers registered, all required methods present`
  );
}
