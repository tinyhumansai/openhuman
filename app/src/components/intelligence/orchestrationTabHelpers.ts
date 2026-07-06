/**
 * Pure helpers for the TinyPlace Orchestration tab — time/label formatting,
 * contact-address resolution, and derived badge keys. Extracted so the tab
 * container and its presentational siblings share one implementation.
 */
import type {
  ContactRequestsResponse,
  ContactView,
} from '../../lib/agentworld/invokeApiClient';
import type { ChatWindow } from '../../lib/orchestration/useOrchestrationChats';

export function formatTime(timestamp: string | null): string {
  if (!timestamp) return '';
  const parsed = Date.parse(timestamp);
  if (!Number.isFinite(parsed)) return '';
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  }).format(new Date(parsed));
}

export function truncate(text: string, length = 96): string {
  if (text.length <= length) return text;
  return `${text.slice(0, length - 1)}…`;
}

export function chatTime(chat: ChatWindow): number {
  if (!chat.lastTimestamp) return 0;
  const parsed = Date.parse(chat.lastTimestamp);
  return Number.isFinite(parsed) ? parsed : 0;
}

// The counterpart agent address for a contact view (request or accepted
// contact). The relay's `/contacts` and `/contacts/requests` payloads do not
// always populate the top-level `agentId`, so fall back to the underlying
// contact record: when we are the addressee the counterpart is the
// `requester`, otherwise it is the `addressee`.
export function contactAddress(view: ContactView): string {
  if (view.agentId) return view.agentId;
  const contact = view.contact;
  if (!contact) return '';
  return view.direction === 'outgoing' ? (contact.addressee ?? '') : (contact.requester ?? '');
}

export function acceptedContactIds(contacts: ContactView[]): Set<string> {
  return new Set(
    contacts
      .filter(contact => contact.status === 'accepted')
      .map(contactAddress)
      .filter(Boolean)
  );
}

export function pendingContactIds(requests: ContactRequestsResponse): Set<string> {
  return new Set(
    [...requests.incoming, ...requests.outgoing]
      .filter(contact => contact.status === 'pending')
      .map(contactAddress)
      .filter(Boolean)
  );
}

export function contactBadgeKey(
  chat: ChatWindow,
  accepted: Set<string>,
  pending: Set<string>
): string | null {
  if (chat.pinned || !chat.peerAgentId) return null;
  if (accepted.has(chat.peerAgentId)) return 'tinyplaceOrchestration.pairing.linked';
  if (pending.has(chat.peerAgentId)) return 'tinyplaceOrchestration.pairing.pending';
  return 'tinyplaceOrchestration.pairing.unlinked';
}

/** Best-effort `@handle` for a tiny.place agent id (cryptoId) from a directory
 * reverse lookup — the registered username if any, else null. The raw address
 * is always shown; the handle is additive. */
export function extractHandle(res: {
  agents?: Array<{ username?: string }>;
  identities?: unknown[];
}): string | null {
  const fromAgent = res.agents?.find(a => a.username)?.username;
  const fromIdentity = (res.identities as Array<{ username?: string }> | undefined)?.find(
    identity => identity?.username
  )?.username;
  const username = fromAgent ?? fromIdentity;
  return username ? username.replace(/^@+/, '') : null;
}
