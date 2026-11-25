/**
 * Timeline-row formatting for tool calls.
 *
 * Every label, icon and category is resolved by the tool presentation
 * registry (`features/conversations/tools/toolPresentation.ts`); this module
 * adapts {@link ToolTimelineEntry} rows onto it and keeps the timeline-only
 * helpers (processing blocks, sources, envelope stripping).
 */
import {
  extractSearchProvider,
  parseWebSearchResult,
} from '../features/conversations/tools/parseWebSearchResult';
import { fillPlaceholders } from '../features/conversations/tools/toolPhrases';
import {
  describeToolCall,
  type ToolCallPresentation,
  type ToolCategory,
  toolLabel,
  type Translate,
} from '../features/conversations/tools/toolPresentation';
import type { ToolTimelineEntry } from '../store/chatRuntimeSlice';
import type { PersistedTranscriptItem } from '../types/turnState';

export type { ToolCategory, Translate };
export { extractSearchProvider };

/** Resolve a timeline row through the registry. */
export function presentTimelineEntry(entry: ToolTimelineEntry): ToolCallPresentation {
  return describeToolCall({
    name: entry.name,
    args: entry.argsBuffer,
    status: entry.status,
    serverLabel: entry.displayName,
    serverDetail: entry.detail,
    toolkitHint: entry.sourceToolName,
  });
}

/**
 * Present-tense label for a bare tool name ("Searching the web"). Used where
 * only the name is known: sub-agent child rows and the mascot's activity line.
 */
export function formatToolName(toolName: string | undefined, t?: Translate): string {
  if (!toolName) return '';
  return toolLabel(describeToolCall({ name: toolName, status: 'running' }), t);
}

/**
 * Whether the registry describes this tool on its own. For these the client
 * label is authoritative and a server `display_label` is ignored; the server
 * label wins only for dynamic tools the registry cannot know.
 */
export function isKnownClientTool(name: string): boolean {
  const { source } = describeToolCall({ name });
  return source !== 'server' && source !== 'fallback';
}

/**
 * Strip `<tool_call>…</tool_call>` envelopes that some models emit inline in
 * their visible / reasoning text. The structured call is already surfaced as
 * its own timeline row, so the raw envelope is pure noise in displayed prose.
 * Also removes a trailing, still-streaming unclosed `<tool_call>…` so a
 * half-arrived delta never flashes raw markup. Whitespace is left intact —
 * callers that render single-line previews collapse it themselves.
 */
export function stripToolCallEnvelopes(text: string | undefined | null): string {
  if (!text) return '';
  return text
    .replace(/<tool_call\b[^>]*>[\s\S]*?<\/tool_call>/gi, '')
    .replace(/<tool_call\b[^>]*>[\s\S]*$/i, '');
}

/** Categorize a (possibly `subagent:`-prefixed) tool name for grouping/icons. */
export function categorizeTool(name: string): ToolCategory {
  return describeToolCall({ name }).category;
}

const STEPS_KEY = {
  one: 'conversations.tools.steps.one',
  other: 'conversations.tools.steps.other',
};
const STEPS_EN = { one: '{count} step', other: '{count} steps' };

/** "3 steps" in the caller's locale. */
export function formatStepCount(count: number, t?: Translate): string {
  const form = count === 1 ? 'one' : 'other';
  const template = t ? t(STEPS_KEY[form], STEPS_EN[form]) : STEPS_EN[form];
  return fillPlaceholders(template, { count: String(count) });
}

/**
 * Summarize a group of tool rows for a timeline header.
 *
 * One row reads as that row's own label. Several read as a step count plus
 * the distinct things done, most frequent first, e.g. "6 steps · Read file
 * ×3, Searched the web ×2, Ran command". Labels come from the registry, so
 * the summary is translated with the rows and never invents a category
 * phrase that disagrees with them.
 */
export function summarizeToolGroup(entries: ToolTimelineEntry[], t?: Translate): string {
  if (entries.length === 0) return '';
  if (entries.length === 1) return formatTimelineEntry(entries[0], t).title;
  return summarizeToolCalls(entries.map(presentTimelineEntry), t);
}

/**
 * The multi-step summary over already-resolved presentations; shared by the
 * processing panel and the chat's tool timeline header.
 */
export function summarizeToolCalls(presentations: ToolCallPresentation[], t?: Translate): string {
  if (presentations.length === 0) return '';
  if (presentations.length === 1) return toolLabel(presentations[0], t);
  const counts = new Map<string, number>();
  for (const presentation of presentations) {
    const label = toolLabel({ ...presentation, tense: 'done' }, t);
    counts.set(label, (counts.get(label) ?? 0) + 1);
  }
  const parts = [...counts.entries()]
    .sort((a, b) => b[1] - a[1])
    .slice(0, 3)
    .map(([label, n]) => (n > 1 ? `${label} ×${n}` : label));
  if (counts.size > 3) parts.push('…');
  return `${formatStepCount(presentations.length, t)} · ${parts.join(', ')}`;
}

/**
 * Title and detail for one timeline row. The title is tense-aware ("Reading
 * file" while running, "Read file" once settled); the detail is the row's
 * target, or for a delegation the full prompt the agent was given.
 */
export function formatTimelineEntry(
  entry: ToolTimelineEntry,
  t?: Translate
): { title: string; detail?: string } {
  const presentation = presentTimelineEntry(entry);
  const title = toolLabel(presentation, t);
  if (presentation.category === 'agent' || presentation.source === 'agent') {
    // A delegation's detail is the whole brief the agent was given, not a
    // capped chip: the rail shows it under the row.
    return {
      title,
      detail: entry.detail ?? promptFromArgsBuffer(entry.argsBuffer) ?? presentation.chip,
    };
  }
  return { title, detail: presentation.chip ?? entry.detail };
}

/**
 * A render block for the "View processing" panel — either a prose block
 * (the agent's narration or hidden reasoning) or a group of consecutive
 * tool rows under a summary. {@link buildProcessingBlocks} derives an
 * ordered list of these from the interleaved transcript.
 */
type ProcessingBlock =
  | { kind: 'narration'; key: string; text: string }
  | { kind: 'thinking'; key: string; text: string; startedAt?: number; endedAt?: number }
  | { kind: 'toolGroup'; key: string; summary: string; entries: ToolTimelineEntry[] };

/**
 * Turn the ordered transcript (narration / thinking / tool-call pointers)
 * plus the tool timeline into the interleaved render model: prose flows
 * inline, and runs of consecutive tool calls collapse into one group with a
 * summary header. Tool pointers are resolved against `entries` by id;
 * unknown ids are skipped. Pure + deterministic for unit testing.
 *
 * When `transcript` is empty (legacy snapshot / pre-streaming row), returns a
 * single tool group over all `entries` so the caller still renders the rows.
 */
export function buildProcessingBlocks(
  transcript: PersistedTranscriptItem[],
  entries: ToolTimelineEntry[],
  t?: Translate
): ProcessingBlock[] {
  const byId = new Map(entries.map(e => [e.id, e]));

  if (transcript.length === 0) {
    return entries.length > 0
      ? [{ kind: 'toolGroup', key: 'all', summary: summarizeToolGroup(entries, t), entries }]
      : [];
  }

  const ordered = [...transcript].sort((a, b) => a.seq - b.seq);
  const blocks: ProcessingBlock[] = [];
  let group: ToolTimelineEntry[] = [];

  const flush = () => {
    if (group.length === 0) return;
    blocks.push({
      kind: 'toolGroup',
      key: `tg-${group[0].id}`,
      summary: summarizeToolGroup(group, t),
      entries: group,
    });
    group = [];
  };

  for (const item of ordered) {
    if (item.kind === 'toolCall') {
      const entry = byId.get(item.callId);
      if (entry) group.push(entry);
      continue;
    }
    // A prose item ends the current tool group.
    flush();
    const text = stripToolCallEnvelopes(item.text).trim();
    if (!text) continue;
    const key = `${item.kind}-${item.seq}`;
    if (item.kind === 'thinking') {
      // Carry the block's timing through so the rail's reasoning panel can
      // say "Thought for Ns".
      blocks.push({
        kind: 'thinking',
        key,
        text,
        ...(item.startedAt == null ? {} : { startedAt: item.startedAt }),
        ...(item.endedAt == null ? {} : { endedAt: item.endedAt }),
      });
    } else {
      blocks.push({ kind: item.kind, key, text });
    }
  }
  flush();
  return blocks;
}

export function promptFromArgsBuffer(argsBuffer?: string): string | undefined {
  const prompt = parseArgsObject(argsBuffer)?.prompt;
  return typeof prompt === 'string' ? prompt.trim() || undefined : undefined;
}

/** A web source an agent fetched, browsed or found during a run. */
export interface AgentSource {
  /** Stable id (the originating timeline entry id, plus a hit index for searches). */
  id: string;
  /** Display title — the page title for a search hit, else the URL hostname. */
  title: string;
  /** Full URL. */
  url: string;
}

/** Tools whose `url` arg represents a real web source the agent visited. */
const URL_SOURCE_TOOLS = new Set([
  'web_fetch',
  'http_request',
  'curl',
  'browser',
  'browser_open',
  'tinyfish_fetch',
  'gitbooks_get_page',
]);

/**
 * Extract the distinct web sources an agent run touched, for the sources
 * list under an answer. Two kinds, both from real data, never fabricated:
 * the `url` argument of fetch/browse calls, and the hits a completed web
 * search returned. Deduplicated by URL, first-seen order.
 */
export function extractAgentSources(entries: ToolTimelineEntry[]): AgentSource[] {
  const seen = new Set<string>();
  const sources: AgentSource[] = [];
  const add = (source: AgentSource) => {
    // `url` is model- or provider-supplied — prompt-injection-influenceable
    // and not guaranteed to be a real web address. Only http(s) sources may
    // reach an `<a href>`, so a `javascript:` / `data:` / `file:` value never
    // becomes clickable.
    if (!source.url || seen.has(source.url) || !isHttpUrl(source.url)) return;
    seen.add(source.url);
    sources.push(source);
  };
  for (const entry of entries) {
    const presentation = presentTimelineEntry(entry);
    if (presentation.body === 'webSearch' && entry.status === 'success') {
      const parsed = parseWebSearchResult(entry.result, entry.structured);
      parsed?.results.forEach((hit, index) =>
        add({ id: `${entry.id}#${index}`, title: hit.title, url: hit.url })
      );
      // An answer call's citations are sources too.
      parsed?.citations?.forEach((hit, index) =>
        add({ id: `${entry.id}#c${index}`, title: hit.title, url: hit.url })
      );
      continue;
    }
    if (entry.status !== 'success' || !URL_SOURCE_TOOLS.has(presentation.baseName)) continue;
    const url = parseArgsObject(entry.argsBuffer)?.url;
    if (typeof url !== 'string') continue;
    const trimmed = url.trim();
    add({ id: entry.id, title: hostnameFromUrl(trimmed) ?? trimmed, url: trimmed });
  }
  return sources;
}

function hostnameFromUrl(url: string): string | undefined {
  try {
    return new URL(url).hostname;
  } catch {
    return undefined;
  }
}

/** True only for well-formed http(s) URLs — the schemes safe to link out to. */
function isHttpUrl(url: string): boolean {
  try {
    const { protocol } = new URL(url);
    return protocol === 'http:' || protocol === 'https:';
  } catch {
    return false;
  }
}

function parseArgsObject(argsBuffer?: string): Record<string, unknown> | null {
  if (!argsBuffer) return null;
  try {
    const parsed: unknown = JSON.parse(argsBuffer);
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed)
      ? (parsed as Record<string, unknown>)
      : null;
  } catch {
    return null;
  }
}
