/**
 * Frontend client for the agent's native tool registry, exposed over
 * `openhuman.javascript_list_tools` (the same registry the assistant uses). The
 * flows "Tool" node (native OpenHuman tools, as opposed to the Composio "App
 * action" node) uses this to offer a dropdown of real tool names + descriptions.
 */
import debug from 'debug';

import { callCoreRpc } from '../coreRpcClient';

const log = debug('runtimeToolsApi');

/** One agent-callable tool (`RuntimeToolSummary` on the Rust side). */
export interface RuntimeTool {
  name: string;
  description: string;
  category: string;
  permission_level: string;
  scope: string;
  supports_markdown: boolean;
  /** JSON-schema-ish parameters object for the tool's args. */
  parameters: unknown;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  return value as Record<string, unknown>;
}

/** Peel the `{ result, logs }` CLI envelope, like `flowsApi` does. */
function unwrapCliEnvelope<T>(payload: unknown): T {
  const record = asRecord(payload);
  if (record && 'result' in record && 'logs' in record && Array.isArray(record.logs)) {
    return record.result as T;
  }
  return payload as T;
}

/**
 * List the native agent tools available to a flow's "Tool" node. The payload is
 * `{ tools: RuntimeTool[] }` (or a bare array on some cores); both are handled.
 */
export async function listRuntimeTools(): Promise<RuntimeTool[]> {
  log('listRuntimeTools: request');
  const response = await callCoreRpc<unknown>({
    method: 'openhuman.javascript_list_tools',
    params: {},
  });
  const payload = unwrapCliEnvelope<unknown>(response);
  const record = asRecord(payload);
  const tools = Array.isArray(payload)
    ? (payload as RuntimeTool[])
    : record && Array.isArray(record.tools)
      ? (record.tools as RuntimeTool[])
      : [];
  log('listRuntimeTools: response count=%d', tools.length);
  return tools;
}
