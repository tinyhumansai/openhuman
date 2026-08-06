export function formatRelativeTime(dateStr: string): string {
  const now = Date.now();
  const then = new Date(dateStr).getTime();
  const diffMs = now - then;
  if (diffMs < 60_000) return 'just now';
  const mins = Math.floor(diffMs / 60_000);
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

export function isAllowedExternalHref(rawHref: string): boolean {
  try {
    const url = new URL(rawHref);
    return url.protocol === 'http:' || url.protocol === 'https:' || url.protocol === 'mailto:';
  } catch {
    return false;
  }
}

/**
 * Custom inline tag any agent can drop inside a chat bubble to render
 * an in-app navigation pill, e.g.
 *
 *     <openhuman-link path="settings/notifications">Allow notifications</openhuman-link>
 *
 * The conversation UI (`AgentMessageBubble`) parses these out of the
 * raw text, splitting the message into ordered text/link segments.
 * Text segments still render through Markdown; link segments render as
 * a clickable pill that calls `react-router`'s navigate(`/${path}`) on
 * click — no deep-link round-trip, no host browser involvement.
 *
 * Path is the hash route under HashRouter (e.g. `settings/notifications`
 * → `#/settings/notifications`). Leading/trailing slashes are tolerated.
 */
interface OpenhumanLinkSegment {
  kind: 'link';
  path: string;
  label: string;
}

interface TextSegment {
  kind: 'text';
  text: string;
}

type BubbleSegment = TextSegment | OpenhumanLinkSegment;

const OPENHUMAN_LINK_RE =
  /<openhuman-link\s+path=(?:"([^"]+)"|'([^']+)')\s*>([\s\S]*?)<\/openhuman-link>/gi;

export function parseBubbleSegments(content: string): BubbleSegment[] {
  if (!content || !content.includes('<openhuman-link')) {
    return [{ kind: 'text', text: content }];
  }
  const segments: BubbleSegment[] = [];
  let cursor = 0;
  // Reset regex state between calls (the global flag preserves lastIndex).
  OPENHUMAN_LINK_RE.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = OPENHUMAN_LINK_RE.exec(content)) !== null) {
    if (match.index > cursor) {
      segments.push({ kind: 'text', text: content.slice(cursor, match.index) });
    }
    const path = (match[1] ?? match[2] ?? '').trim().replace(/^\/+/, '').replace(/\/+$/, '');
    const label = (match[3] ?? '').trim();
    if (path && label) {
      segments.push({ kind: 'link', path, label });
    }
    cursor = match.index + match[0].length;
  }
  if (cursor < content.length) {
    segments.push({ kind: 'text', text: content.slice(cursor) });
  }
  return segments;
}

export type AgentBubblePosition = 'single' | 'first' | 'middle' | 'last';

export function getAgentBubbleChrome(position: AgentBubblePosition): string {
  if (position === 'single') return 'rounded-2xl rounded-bl-md';
  if (position === 'first') return 'rounded-2xl rounded-bl-lg';
  if (position === 'middle') return 'rounded-2xl rounded-tl-md rounded-bl-lg';
  return 'rounded-2xl rounded-tl-md rounded-bl-md';
}

export function formatResetTime(isoStr: string): string {
  const ms = new Date(isoStr).getTime() - Date.now();
  if (ms <= 0) return 'now';
  const mins = Math.ceil(ms / 60_000);
  if (mins < 60) return `in ${mins}m`;
  const hours = Math.floor(mins / 60);
  const remMins = mins % 60;
  if (hours < 24) return remMins > 0 ? `in ${hours}h ${remMins}m` : `in ${hours}h`;
  const days = Math.floor(hours / 24);
  const remHours = hours % 24;
  return remHours > 0 ? `in ${days}d ${remHours}h` : `in ${days}d`;
}
