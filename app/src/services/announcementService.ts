/**
 * Announcement service
 *
 * Thin RPC wrapper around the core `announcements_get_latest` controller, plus
 * parsing into a typed announcement. The core proxies the backend's
 * `GET /announcements/latest`, which returns the announcement object or, when
 * nothing qualifies, an empty/`null` payload. We treat "no string id" as "no
 * announcement" so the UI never renders an empty banner.
 */
import { callCoreRpc } from './coreRpcClient';

export type AnnouncementSeverity = 'INFO' | 'WARNING' | 'CRITICAL';

export interface AnnouncementCta {
  label: string;
  url: string;
}

export interface Announcement {
  id: string;
  title: string;
  body: string;
  severity: AnnouncementSeverity;
  cta: AnnouncementCta | null;
  startsAt: string | null;
  expiresAt: string | null;
  createdAt: string | null;
}

const SEVERITIES: AnnouncementSeverity[] = ['INFO', 'WARNING', 'CRITICAL'];

function asString(value: unknown): string | null {
  return typeof value === 'string' && value.length > 0 ? value : null;
}

function parseCta(raw: unknown): AnnouncementCta | null {
  if (!raw || typeof raw !== 'object') {
    return null;
  }
  const data = raw as Record<string, unknown>;
  const label = asString(data.label);
  const url = asString(data.url);
  if (!label || !url) {
    return null;
  }
  return { label, url };
}

export function parseAnnouncement(raw: unknown): Announcement | null {
  if (!raw || typeof raw !== 'object') {
    return null;
  }
  const data = raw as Record<string, unknown>;
  const id = asString(data.id) ?? asString(data._id);
  const title = asString(data.title);
  const body = asString(data.body);
  // The backend collapses "no active announcement" to an empty object; without
  // an id (and the core fields) there is nothing to show.
  if (!id || !title || !body) {
    return null;
  }
  const severity = asString(data.severity) as AnnouncementSeverity | null;
  return {
    id,
    title,
    body,
    severity: severity && SEVERITIES.includes(severity) ? severity : 'INFO',
    cta: parseCta(data.cta),
    startsAt: asString(data.startsAt),
    expiresAt: asString(data.expiresAt),
    createdAt: asString(data.createdAt),
  };
}

/**
 * Fetch the latest active announcement, or null. Network/auth failures resolve
 * to null — a missing announcement is never worth surfacing an error for.
 */
export async function fetchLatestAnnouncement(): Promise<Announcement | null> {
  const payload = await callCoreRpc<unknown>({
    method: 'openhuman.announcements_get_latest',
    timeoutMs: 8000,
  });
  return parseAnnouncement(payload);
}
