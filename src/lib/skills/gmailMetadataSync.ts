/**
 * Send Gmail profile (and optionally emails) to the backend via the
 * `integration:metadata-sync` socket event so the server can merge them
 * into the user's Google OAuth integration metadata.
 */
import { emitViaRustSocket } from '../../utils/tauriSocket';

const INTEGRATION_METADATA_SYNC_EVENT = 'integration:metadata-sync';
const PROVIDER_GOOGLE = 'google';

/** Gmail profile shape from skill state (snake_case). */
interface GmailProfileLike {
  email_address: string;
  messages_total: number;
  threads_total: number;
  history_id: string;
}

/** Single email summary from skill state. */
interface GmailEmailSummaryLike {
  id: string;
  threadId: string;
  snippet?: string;
  subject?: string;
  from?: string;
  date?: string;
}

/** Gmail skill state slice we care about for metadata sync. */
export interface GmailStateForSync {
  profile?: GmailProfileLike | null;
  emails?: GmailEmailSummaryLike[] | null;
}

/**
 * Emit `integration:metadata-sync` with Gmail profile and emails so the
 * backend can merge them into the user's Google OAuth integration.
 * No-op when profile is missing or not in Tauri.
 */
export function syncGmailMetadataToBackend(gmailState: GmailStateForSync | undefined): void {
  if (!gmailState?.profile || typeof gmailState.profile !== 'object') return;

  const profile = gmailState.profile as GmailProfileLike;
  const metadata: Record<string, unknown> = {
    email_address: profile.email_address,
    messages_total: profile.messages_total,
    threads_total: profile.threads_total,
    history_id: profile.history_id,
  };

  if (Array.isArray(gmailState.emails) && gmailState.emails.length > 0) {
    metadata.emails = gmailState.emails as GmailEmailSummaryLike[];
  }

  const payload = {
    requestId: crypto.randomUUID(),
    provider: PROVIDER_GOOGLE,
    metadata,
  };

  void emitViaRustSocket(INTEGRATION_METADATA_SYNC_EVENT, payload);
}
