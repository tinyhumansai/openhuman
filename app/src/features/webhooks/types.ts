import type { Tunnel } from '../../services/api/tunnelsApi';

export type { Tunnel };

export interface TunnelRegistration {
  tunnel_uuid: string;
  target_kind?: string;
  skill_id: string;
  tunnel_name: string | null;
  backend_tunnel_id: string | null;
}

export interface WebhookActivityEntry {
  correlation_id: string;
  tunnel_name: string;
  method: string;
  path: string;
  status_code: number | null;
  skill_id: string | null;
  timestamp: number;
}
