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

export function getInlineCompletionSuffix(input: string, suggestion: string): string {
  if (!input || !suggestion) return '';
  const normalize = (value: string) =>
    value
      .replace(/\u2192/g, ' ')
      .replace(/\s+/g, ' ')
      .trim();

  const normalizedInput = normalize(input);
  const normalizedSuggestion = normalize(suggestion);
  if (!normalizedSuggestion) return '';

  if (normalizedSuggestion.startsWith(normalizedInput)) {
    return normalizedSuggestion.slice(normalizedInput.length).trimStart();
  }

  const maxOverlap = Math.min(normalizedInput.length, normalizedSuggestion.length, 120);
  for (let overlap = maxOverlap; overlap >= 1; overlap -= 1) {
    if (
      normalizedInput.slice(normalizedInput.length - overlap) ===
      normalizedSuggestion.slice(0, overlap)
    ) {
      return normalizedSuggestion.slice(overlap).trimStart();
    }
  }

  if (normalizedInput.endsWith(normalizedSuggestion)) {
    return '';
  }
  return normalizedSuggestion;
}

export function buildAcceptedInlineCompletion(input: string, suffix: string): string {
  const normalizedInput = input.replace(/\u2192/g, ' ').replace(/\t+/g, ' ');
  const cleanSuffix = suffix
    .replace(/\u2192/g, ' ')
    .replace(/\t+/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();

  if (!cleanSuffix) return normalizedInput;

  const needsSpace =
    normalizedInput.length > 0 && !/\s$/.test(normalizedInput) && !/^[,.;:!?)]/.test(cleanSuffix);

  return `${normalizedInput}${needsSpace ? ' ' : ''}${cleanSuffix}`;
}

export function isAllowedExternalHref(rawHref: string): boolean {
  try {
    const url = new URL(rawHref);
    return url.protocol === 'http:' || url.protocol === 'https:' || url.protocol === 'mailto:';
  } catch {
    return false;
  }
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
