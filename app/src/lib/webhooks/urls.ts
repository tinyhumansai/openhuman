import { BACKEND_URL } from '../../utils/config';

const DEFAULT_BACKEND_URL = 'https://api.tinyhumans.ai';

function normalizedBackendUrl(baseUrl?: string): string {
  const value = (baseUrl || BACKEND_URL || DEFAULT_BACKEND_URL).trim();
  return value.replace(/\/+$/, '');
}

export function buildWebhookIngressUrl(tunnelUuid: string, baseUrl?: string): string {
  return `${normalizedBackendUrl(baseUrl)}/webhooks/ingress/${encodeURIComponent(tunnelUuid)}`;
}
