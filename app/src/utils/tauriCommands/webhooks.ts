/**
 * Webhook debug commands.
 */
import { callCoreRpc } from '../../services/coreRpcClient';
import { isTauri, CommandResponse } from './common';

export interface WebhookDebugRegistration {
  tunnel_uuid: string;
  target_kind: string;
  skill_id: string;
  tunnel_name: string | null;
  backend_tunnel_id: string | null;
}

export interface WebhookDebugLogEntry {
  correlation_id: string;
  tunnel_id: string;
  tunnel_uuid: string;
  tunnel_name: string;
  method: string;
  path: string;
  skill_id: string | null;
  status_code: number | null;
  timestamp: number;
  updated_at: number;
  request_headers: Record<string, unknown>;
  request_query: Record<string, string>;
  request_body: string;
  response_headers: Record<string, string>;
  response_body: string;
  stage: string;
  error_message: string | null;
  raw_payload?: unknown;
}

export interface WebhookDebugEvent {
  event_type: string;
  timestamp: number;
  correlation_id?: string | null;
  tunnel_uuid?: string | null;
}

export async function openhumanWebhooksListRegistrations(): Promise<
  CommandResponse<{ result: { registrations: WebhookDebugRegistration[] } }>
> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<
    CommandResponse<{ result: { registrations: WebhookDebugRegistration[] } }>
  >({ method: 'openhuman.webhooks_list_registrations' });
}

export async function openhumanWebhooksListLogs(
  limit = 100
): Promise<CommandResponse<{ result: { logs: WebhookDebugLogEntry[] } }>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<{ result: { logs: WebhookDebugLogEntry[] } }>>({
    method: 'openhuman.webhooks_list_logs',
    params: { limit },
  });
}

export async function openhumanWebhooksClearLogs(): Promise<CommandResponse<{ cleared: number }>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<{ cleared: number }>>({
    method: 'openhuman.webhooks_clear_logs',
  });
}

export async function openhumanWebhooksRegisterEcho(
  tunnelUuid: string,
  tunnelName?: string,
  backendTunnelId?: string
): Promise<CommandResponse<{ result: { registrations: WebhookDebugRegistration[] } }>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<
    CommandResponse<{ result: { registrations: WebhookDebugRegistration[] } }>
  >({
    method: 'openhuman.webhooks_register_echo',
    params: {
      tunnel_uuid: tunnelUuid,
      tunnel_name: tunnelName ?? null,
      backend_tunnel_id: backendTunnelId ?? null,
    },
  });
}

export async function openhumanWebhooksUnregisterEcho(
  tunnelUuid: string
): Promise<CommandResponse<{ result: { registrations: WebhookDebugRegistration[] } }>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<
    CommandResponse<{ result: { registrations: WebhookDebugRegistration[] } }>
  >({ method: 'openhuman.webhooks_unregister_echo', params: { tunnel_uuid: tunnelUuid } });
}
