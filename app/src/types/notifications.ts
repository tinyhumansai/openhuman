//! TypeScript types mirroring the Rust `notifications` domain types.

export type NotificationStatus = 'unread' | 'read' | 'acted' | 'dismissed';

export interface IntegrationNotification {
  id: string;
  /** Provider slug: "gmail", "slack", "whatsapp", etc. */
  provider: string;
  account_id?: string;
  title: string;
  body: string;
  raw_payload: Record<string, unknown>;
  /** 0.0–1.0 importance score from the triage pipeline (undefined until scored). */
  importance_score?: number;
  /** Triage action: "drop" | "acknowledge" | "react" | "escalate" */
  triage_action?: string;
  /** One-sentence justification from the classifier. */
  triage_reason?: string;
  status: NotificationStatus;
  /** ISO 8601 timestamp */
  received_at: string;
  /** ISO 8601 timestamp — undefined until triage completes */
  scored_at?: string;
}

export interface NotificationSettings {
  provider: string;
  enabled: boolean;
  /** Minimum importance score 0.0–1.0 to show; 0.0 = show all */
  importance_threshold: number;
  route_to_orchestrator: boolean;
}

export interface NotificationStats {
  total: number;
  unread: number;
  unscored: number;
  by_provider: Record<string, number>;
  by_action: Record<string, number>;
}
