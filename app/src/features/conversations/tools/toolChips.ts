/**
 * Chip extractors: the short target shown beside a tool's label ("Read file
 * `…/src/main.ts`", "Searched the web `rust async traits`").
 *
 * Every value here comes from a model-emitted argument, so it is treated as
 * untrusted display text: trimmed, whitespace-collapsed and length-capped.
 * Nothing in this file produces a link.
 */
import { truncateText } from '../../../utils/truncateText';

export type ToolArgs = Record<string, unknown>;
export type ChipRule = (args: ToolArgs) => string | undefined;

const MAX_CHIP_LENGTH = 80;

export function truncateChip(value: string, max = MAX_CHIP_LENGTH): string {
  const cleaned = value.trim().replace(/\s+/g, ' ');
  return truncateText(cleaned, max);
}

function stringArg(args: ToolArgs, key: string): string | undefined {
  const value = args[key];
  if (typeof value === 'string' && value.trim()) return value;
  if (typeof value === 'number' && Number.isFinite(value)) return String(value);
  return undefined;
}

/** First non-empty string among `keys`. */
export function firstArg(args: ToolArgs, ...keys: string[]): string | undefined {
  for (const key of keys) {
    const value = stringArg(args, key);
    if (value) return value;
  }
  return undefined;
}

/** `/a/b/c/d.ts` → `…/c/d.ts`; short paths pass through. */
export function shortenPath(filePath: string): string {
  const parts = filePath.split('/');
  if (parts.length <= 3) return filePath;
  return `…/${parts.slice(-2).join('/')}`;
}

/** `https://docs.rs/tokio/latest/x` → `docs.rs/tokio/latest/x`, capped. */
export function displayUrl(url: string): string {
  try {
    const parsed = new URL(url);
    const path = parsed.pathname === '/' ? '' : parsed.pathname;
    return truncateChip(`${parsed.hostname}${path}`);
  } catch {
    return truncateChip(url);
  }
}

export function hostnameOf(url: string): string | undefined {
  try {
    return new URL(url).hostname || undefined;
  } catch {
    return undefined;
  }
}

/** Rule factories, so the spec tables stay declarative. */
export const chip = {
  text:
    (...keys: string[]): ChipRule =>
    args => {
      const value = firstArg(args, ...keys);
      return value ? truncateChip(value) : undefined;
    },
  path:
    (...keys: string[]): ChipRule =>
    args => {
      const value = firstArg(args, ...(keys.length ? keys : ['path', 'file_path']));
      return value ? truncateChip(shortenPath(value.trim())) : undefined;
    },
  url:
    (...keys: string[]): ChipRule =>
    args => {
      const value = firstArg(args, ...(keys.length ? keys : ['url', 'uri']));
      if (value) return displayUrl(value.trim());
      const list = args.urls;
      if (Array.isArray(list) && typeof list[0] === 'string') {
        const first = displayUrl(list[0]);
        return list.length > 1 ? `${first} +${list.length - 1}` : first;
      }
      return undefined;
    },
  query: (): ChipRule => args => {
    const value = firstArg(args, 'query', 'q', 'search_query', 'objective');
    if (value) return truncateChip(value);
    const queries = args.search_queries;
    if (Array.isArray(queries) && typeof queries[0] === 'string') return truncateChip(queries[0]);
    return undefined;
  },
  command:
    (...keys: string[]): ChipRule =>
    args => {
      const value = firstArg(args, ...(keys.length ? keys : ['command']));
      return value ? truncateChip(value, 120) : undefined;
    },
  /** First path among a multi-edit payload (`apply_patch { edits: [{ path }] }`). */
  editsPath: (): ChipRule => args => {
    const edits = args.edits;
    if (!Array.isArray(edits) || edits.length === 0) return firstArg(args, 'path');
    const first = edits[0] as ToolArgs | undefined;
    const path = first && typeof first.path === 'string' ? shortenPath(first.path) : undefined;
    if (!path) return undefined;
    return edits.length > 1 ? `${path} +${edits.length - 1}` : path;
  },
};

/**
 * Generic chip for a tool the tables do not describe: the same key order the
 * core's `context_detail_from_args` walks, so an unknown tool still shows its
 * obvious target.
 */
export const genericChip: ChipRule = args => {
  const value = firstArg(
    args,
    'to',
    'recipient',
    'email',
    'query',
    'q',
    'url',
    'file_path',
    'path',
    'command',
    'subject',
    'title',
    'channel',
    'repo',
    'name'
  );
  return value ? truncateChip(value) : undefined;
};
