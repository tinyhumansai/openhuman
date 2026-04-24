/**
 * ConnectionBadge — small pill badge rendered on connection cards.
 *
 * Two kinds:
 *   - "Messaging"  — shown for iMessage, Telegram, WhatsApp channel cards
 *   - "Composio"   — shown for cards backed by the Composio toolkit (kind === 'composio')
 */

const MESSAGING_IDS = ['imessage', 'telegram', 'whatsapp'] as const;
type MessagingId = (typeof MESSAGING_IDS)[number];

export function isMessagingId(id: string): id is MessagingId {
  return (MESSAGING_IDS as readonly string[]).includes(id);
}

interface ConnectionBadgeProps {
  /** 'composio' | 'messaging' */
  kind: 'composio' | 'messaging';
}

export default function ConnectionBadge({ kind }: ConnectionBadgeProps) {
  if (kind === 'composio') {
    return (
      <span className="inline-flex items-center rounded-full bg-violet-50 border border-violet-200 px-2 py-0.5 text-[10px] font-medium text-violet-700 leading-none">
        Composio
      </span>
    );
  }
  return (
    <span className="inline-flex items-center rounded-full bg-sky-50 border border-sky-200 px-2 py-0.5 text-[10px] font-medium text-sky-700 leading-none">
      Messaging
    </span>
  );
}
