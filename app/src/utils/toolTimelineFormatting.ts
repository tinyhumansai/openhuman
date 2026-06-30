import type { ToolTimelineEntry } from '../store/chatRuntimeSlice';
import type { PersistedTranscriptItem } from '../types/turnState';

interface ParsedToolArgs {
  agent_id?: string;
  prompt?: string;
  toolkit?: string;
  command?: string;
  url?: string;
  path?: string;
  file_path?: string;
  pattern?: string;
  query?: string;
  tool_name?: string;
  question?: string;
}

const TOOL_DISPLAY_NAMES: Record<string, string> = {
  shell: 'Running command',
  node_exec: 'Running command',
  npm_exec: 'Running command',
  web_fetch: 'Fetching',
  http_request: 'Fetching',
  curl: 'Fetching',
  web_search: 'Searching the web',
  gitbooks_search: 'Searching docs',
  file_read: 'Reading file',
  file_write: 'Writing file',
  edit: 'Editing file',
  apply_patch: 'Applying patch',
  grep: 'Searching code',
  glob: 'Finding files',
  list: 'Listing directory',
  read_diff: 'Reading diff',
  git_operations: 'Git operation',
  browser: 'Browsing',
  browser_open: 'Opening browser',
  screenshot: 'Taking screenshot',
  image_info: 'Analyzing image',
  install_tool: 'Installing tool',
  lsp: 'Code intelligence',
  keyboard: 'Typing',
  mouse: 'Clicking',
  csv_export: 'Exporting CSV',
  update_memory_md: 'Updating memory',
  read_workspace_state: 'Reading workspace',
  current_time: 'Checking time',
  schedule: 'Scheduling',
  detect_tools: 'Detecting tools',
  tool_stats: 'Tool statistics',
  vault_write_markdown: 'Writing to vault',
  run_linter: 'Running linter',
  run_tests: 'Running tests',
  proxy_config: 'Configuring proxy',
  update_check: 'Checking for updates',
  update_apply: 'Applying update',
  pushover: 'Sending notification',
  insert_sql_record: 'Inserting record',
  mcp_list_servers: 'Listing MCP servers',
  mcp_list_tools: 'Listing MCP tools',
  mcp_call_tool: 'Calling MCP tool',
  mcp_setup_search: 'Searching MCP tools',
  mcp_setup_get: 'Getting MCP tool',
  mcp_setup_install_and_connect: 'Installing MCP server',
  mcp_setup_request_secret: 'Requesting secret',
  mcp_setup_test_connection: 'Testing connection',
  polymarket: 'Checking markets',
  gmail_unsubscribe: 'Unsubscribing',
  gitbooks_get_page: 'Reading docs page',
  audio_generate_podcast: 'Generating podcast',
  audio_email_podcast: 'Emailing podcast',
  audio_generate_and_email_podcast: 'Generating & emailing podcast',
  composio_list_connections: 'Viewing your Connections',
  agent_prepare_context: 'Preparing context',
};

/**
 * Format a raw tool name into a short human-readable label.
 * Used for subagent child tool rows and sub-mascot activity text.
 */
export function formatToolName(toolName: string | undefined): string {
  if (!toolName) return '';
  return TOOL_DISPLAY_NAMES[toolName] ?? humanizeIdentifier(toolName);
}

/**
 * The fixed set of built-in / special tools this client formatter labels
 * well on its own (with args-aware detail). For these, the client label is
 * authoritative and a server-supplied `display_label` is ignored — the
 * server label only wins for *dynamic* tools (Composio/MCP/integration
 * actions) the client can't possibly know, which is where raw `snake_case`
 * used to leak through. Keep in sync with {@link formatTimelineEntry} /
 * {@link formatToolDetail}.
 */
const CLIENT_KNOWN_TOOLS = new Set<string>([
  ...Object.keys(TOOL_DISPLAY_NAMES),
  // args-aware built-ins handled by formatToolDetail()
  'shell',
  'node_exec',
  'npm_exec',
  'web_fetch',
  'http_request',
  'curl',
  'web_search',
  'gitbooks_search',
  'file_read',
  'file_write',
  'vault_write_markdown',
  'edit',
  'apply_patch',
  'grep',
  'glob',
  'list',
  'git_operations',
  'browser',
  'browser_open',
  'screenshot',
  'image_info',
  'install_tool',
  'lsp',
  'run_tests',
  'run_linter',
  'read_diff',
  // special-cased agent / integration rows
  'spawn_subagent',
  'integrations_agent',
  'researcher',
  'agent_prepare_context',
  'context_scout',
  'composio_list_connections',
  'orchestrator',
  'critic',
  'tools_agent',
  'code_executor',
]);

/**
 * Whether the client formatter recognizes this tool (so its label should win
 * over any server-supplied one). True for built-ins, the special agent rows,
 * and the `subagent:` / `delegate_` families that {@link formatTimelineEntry}
 * handles explicitly.
 */
export function isKnownClientTool(name: string): boolean {
  return (
    name.startsWith('subagent:') || name.startsWith('delegate_') || CLIENT_KNOWN_TOOLS.has(name)
  );
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

/** Broad activity category for a tool, used to group + icon timeline rows. */
export type ToolCategory = 'read' | 'write' | 'search' | 'run' | 'fetch' | 'browse' | 'other';

const TOOL_CATEGORIES: Record<string, ToolCategory> = {
  file_read: 'read',
  list: 'read',
  read_diff: 'read',
  file_write: 'write',
  vault_write_markdown: 'write',
  edit: 'write',
  apply_patch: 'write',
  grep: 'search',
  glob: 'search',
  web_search: 'search',
  gitbooks_search: 'search',
  gitbooks_get_page: 'read',
  shell: 'run',
  node_exec: 'run',
  npm_exec: 'run',
  run_tests: 'run',
  run_linter: 'run',
  git_operations: 'run',
  web_fetch: 'fetch',
  http_request: 'fetch',
  curl: 'fetch',
  browser: 'browse',
  browser_open: 'browse',
  screenshot: 'browse',
};

/** Categorize a (possibly `subagent:`-prefixed) tool name for grouping/icons. */
export function categorizeTool(name: string): ToolCategory {
  const base = name.replace(/^subagent:/, '');
  return TOOL_CATEGORIES[base] ?? 'other';
}

/** Plural-aware verb phrase per category, e.g. `read` + 2 → "Read 2 files". */
const CATEGORY_PHRASE: Record<
  ToolCategory,
  { verb: string; noun: [singular: string, plural: string] }
> = {
  read: { verb: 'Read', noun: ['file', 'files'] },
  write: { verb: 'Edited', noun: ['file', 'files'] },
  search: { verb: 'Ran', noun: ['search', 'searches'] },
  run: { verb: 'Ran', noun: ['command', 'commands'] },
  fetch: { verb: 'Fetched', noun: ['page', 'pages'] },
  browse: { verb: 'Browsed', noun: ['page', 'pages'] },
  other: { verb: 'Ran', noun: ['step', 'steps'] },
};

/**
 * Summarize a group of consecutive tool rows into a single Hermes-style
 * header — "Viewed 2 files", "Ran 3 commands", or, for a mixed group, the
 * distinct category phrases joined ("Edited a file, read a file"). A
 * single-row group defers to that row's specific label (more informative
 * than a generic count). Pure + deterministic for unit testing.
 */
export function summarizeToolGroup(entries: ToolTimelineEntry[]): string {
  if (entries.length === 0) return '';
  if (entries.length === 1) {
    return formatTimelineEntry(entries[0]).title;
  }
  // Count per category, preserving first-seen order.
  const order: ToolCategory[] = [];
  const counts = new Map<ToolCategory, number>();
  for (const entry of entries) {
    const cat = categorizeTool(entry.name);
    if (!counts.has(cat)) order.push(cat);
    counts.set(cat, (counts.get(cat) ?? 0) + 1);
  }
  const phrases = order.map((cat, i) => {
    const n = counts.get(cat) ?? 0;
    const { verb, noun } = CATEGORY_PHRASE[cat];
    const word = n === 1 ? noun[0] : noun[1];
    const phrase = `${verb} ${n} ${word}`;
    // Lowercase the leading verb on all but the first phrase so the joined
    // sentence reads naturally ("Edited a file, ran 2 commands").
    return i === 0 ? phrase : phrase.charAt(0).toLowerCase() + phrase.slice(1);
  });
  return phrases.join(', ');
}

export function formatTimelineEntry(entry: ToolTimelineEntry): { title: string; detail?: string } {
  const parsedArgs = parseToolArgs(entry.argsBuffer);

  if (entry.name === 'spawn_subagent' && parsedArgs?.agent_id === 'integrations_agent') {
    const provider =
      inferIntegrationName(parsedArgs.toolkit) ?? inferIntegrationNameFromPrompt(parsedArgs.prompt);
    return {
      title: provider ? integrationActivityTitle(provider) : 'Checking your connected app',
      detail: parsedArgs.prompt?.trim() || entry.detail,
    };
  }

  if (entry.name === 'integrations_agent' || entry.name === 'subagent:integrations_agent') {
    const provider =
      inferIntegrationName(entry.sourceToolName) ??
      inferIntegrationName(parsedArgs?.toolkit) ??
      inferIntegrationNameFromPrompt(entry.detail) ??
      inferIntegrationNameFromPrompt(parsedArgs?.prompt);

    return {
      title: provider ? integrationActivityTitle(provider) : 'Checking your connected app',
      detail: entry.detail,
    };
  }

  if (entry.name === 'subagent:researcher' || entry.name === 'researcher') {
    return { title: 'Researching', detail: entry.detail };
  }
  if (entry.name === 'agent_prepare_context') {
    return { title: 'Preparing context', detail: parsedArgs?.question?.trim() || entry.detail };
  }
  if (entry.name === 'subagent:context_scout' || entry.name === 'context_scout') {
    return { title: 'Scouting context', detail: entry.detail };
  }
  if (entry.name === 'composio_list_connections') {
    return { title: 'Viewing your Connections', detail: entry.detail };
  }
  if (entry.name === 'subagent:orchestrator' || entry.name === 'orchestrator') {
    return { title: 'Planning next steps', detail: entry.detail };
  }
  if (entry.name === 'subagent:critic' || entry.name === 'critic') {
    return { title: 'Reviewing the work', detail: entry.detail };
  }
  if (entry.name === 'subagent:tools_agent' || entry.name === 'tools_agent') {
    return { title: 'Using tools', detail: entry.detail };
  }
  if (entry.name === 'subagent:code_executor' || entry.name === 'code_executor') {
    return { title: 'Running code', detail: entry.detail };
  }

  if (entry.name.startsWith('delegate_')) {
    const provider =
      inferIntegrationName(parsedArgs?.toolkit) ??
      inferIntegrationNameFromPrompt(parsedArgs?.prompt) ??
      inferIntegrationName(entry.name);

    let title: string;
    if (provider) {
      title = integrationActivityTitle(provider);
    } else if (entry.name === 'delegate_to_integrations_agent') {
      const rawToolkit = parsedArgs?.toolkit?.trim();
      title = rawToolkit
        ? integrationActivityTitle(humanizeIdentifier(rawToolkit))
        : 'Checking your connected app';
    } else {
      title = humanizeIdentifier(entry.name);
    }

    return { title, detail: entry.detail ?? parsedArgs?.prompt };
  }

  // ── Tool-specific formatting with args-derived detail ──────────────
  const toolDetail = formatToolDetail(entry.name, parsedArgs);
  if (toolDetail) {
    return { title: toolDetail.title, detail: toolDetail.detail ?? entry.detail };
  }

  return {
    title: entry.displayName ?? humanizeIdentifier(entry.name),
    detail: entry.detail ?? parsedArgs?.prompt,
  };
}

/**
 * A render block for the "View processing" panel — either a prose block
 * (the agent's narration or hidden reasoning) or a group of consecutive
 * tool rows under a Hermes-style summary. {@link buildProcessingBlocks}
 * derives an ordered list of these from the interleaved transcript.
 */
export type ProcessingBlock =
  | { kind: 'narration'; key: string; text: string }
  | { kind: 'thinking'; key: string; text: string }
  | { kind: 'toolGroup'; key: string; summary: string; entries: ToolTimelineEntry[] };

/**
 * Turn the ordered transcript (narration / thinking / tool-call pointers)
 * plus the tool timeline into the interleaved Hermes render model: prose
 * flows inline, and runs of consecutive tool calls collapse into one group
 * with a summary header. Tool pointers are resolved against `entries` by id;
 * unknown ids are skipped. Pure + deterministic for unit testing.
 *
 * When `transcript` is empty (legacy snapshot / pre-streaming row), returns a
 * single tool group over all `entries` so the caller still renders the rows.
 */
export function buildProcessingBlocks(
  transcript: PersistedTranscriptItem[],
  entries: ToolTimelineEntry[]
): ProcessingBlock[] {
  const byId = new Map(entries.map(e => [e.id, e]));

  if (transcript.length === 0) {
    return entries.length > 0
      ? [{ kind: 'toolGroup', key: 'all', summary: summarizeToolGroup(entries), entries }]
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
      summary: summarizeToolGroup(group),
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
    blocks.push({ kind: item.kind, key: `${item.kind}-${item.seq}`, text });
  }
  flush();
  return blocks;
}

export function promptFromArgsBuffer(argsBuffer?: string): string | undefined {
  return parseToolArgs(argsBuffer)?.prompt?.trim() || undefined;
}

/** A web source an agent fetched/browsed during a run. */
export interface AgentSource {
  /** Stable id (the originating timeline entry id). */
  id: string;
  /** Display title — the URL hostname. */
  title: string;
  /** Full URL. */
  url: string;
}

/** Tools whose `url` arg represents a real web source the agent visited. */
const URL_SOURCE_TOOLS = new Set(['web_fetch', 'http_request', 'curl', 'browser', 'browser_open']);

/**
 * Extract the distinct web sources an agent run touched, for the
 * "Agent Process Source" panel. Derived from real `url` args on
 * fetch/browse timeline entries — never fabricated. Deduplicated by URL,
 * preserving first-seen order.
 */
export function extractAgentSources(entries: ToolTimelineEntry[]): AgentSource[] {
  const seen = new Set<string>();
  const sources: AgentSource[] = [];
  for (const entry of entries) {
    const baseName = entry.name.replace(/^subagent:/, '');
    if (!URL_SOURCE_TOOLS.has(baseName)) continue;
    const url = parseToolArgs(entry.argsBuffer)?.url?.trim();
    if (!url || seen.has(url)) continue;
    seen.add(url);
    sources.push({ id: entry.id, title: hostnameFromUrl(url) ?? url, url });
  }
  return sources;
}

const MAX_DETAIL_LEN = 120;

function truncateDetail(value: string): string {
  const cleaned = value.trim().replace(/\s+/g, ' ');
  if (cleaned.length <= MAX_DETAIL_LEN) return cleaned;
  return `${cleaned.slice(0, MAX_DETAIL_LEN - 1)}…`;
}

function hostnameFromUrl(url: string): string | undefined {
  try {
    return new URL(url).hostname;
  } catch {
    return undefined;
  }
}

function shortenPath(filePath: string): string {
  const parts = filePath.split('/');
  if (parts.length <= 3) return filePath;
  return `…/${parts.slice(-2).join('/')}`;
}

function formatToolDetail(
  name: string,
  args: ParsedToolArgs | null
): { title: string; detail?: string } | null {
  switch (name) {
    case 'shell':
    case 'node_exec':
    case 'npm_exec': {
      const cmd = args?.command?.trim();
      return { title: 'Running command', detail: cmd ? truncateDetail(cmd) : undefined };
    }

    case 'web_fetch':
    case 'http_request':
    case 'curl': {
      const url = args?.url?.trim();
      const host = url ? hostnameFromUrl(url) : undefined;
      return {
        title: host ? `Fetching ${host}` : 'Fetching',
        detail: url ? truncateDetail(url) : undefined,
      };
    }

    case 'web_search': {
      const query = args?.query?.trim();
      return { title: query ? `Searching: ${truncateDetail(query)}` : 'Searching the web' };
    }

    case 'gitbooks_search': {
      const query = args?.query?.trim();
      return { title: query ? `Searching docs: ${truncateDetail(query)}` : 'Searching docs' };
    }

    case 'file_read': {
      const p = args?.path?.trim() ?? args?.file_path?.trim();
      return { title: 'Reading file', detail: p ? shortenPath(p) : undefined };
    }

    case 'file_write':
    case 'vault_write_markdown': {
      const p = args?.path?.trim() ?? args?.file_path?.trim();
      return { title: 'Writing file', detail: p ? shortenPath(p) : undefined };
    }

    case 'edit':
    case 'apply_patch': {
      const p = args?.path?.trim() ?? args?.file_path?.trim();
      return { title: 'Editing file', detail: p ? shortenPath(p) : undefined };
    }

    case 'grep': {
      const pat = args?.pattern?.trim();
      return { title: pat ? `Searching: ${truncateDetail(pat)}` : 'Searching code' };
    }

    case 'glob': {
      const pat = args?.pattern?.trim();
      return { title: pat ? `Finding: ${truncateDetail(pat)}` : 'Finding files' };
    }

    case 'list': {
      const p = args?.path?.trim();
      return { title: 'Listing directory', detail: p ? shortenPath(p) : undefined };
    }

    case 'git_operations': {
      const cmd = args?.command?.trim();
      if (cmd) {
        const verb = cmd.split(/\s+/)[0];
        return { title: `Git ${verb}`, detail: truncateDetail(cmd) };
      }
      return { title: 'Git operation' };
    }

    case 'browser':
    case 'browser_open': {
      const url = args?.url?.trim();
      const host = url ? hostnameFromUrl(url) : undefined;
      return { title: host ? `Browsing ${host}` : 'Browsing' };
    }

    case 'screenshot':
      return { title: 'Taking screenshot' };

    case 'image_info':
      return { title: 'Analyzing image' };

    case 'install_tool': {
      const tn = args?.tool_name?.trim();
      return { title: tn ? `Installing ${tn}` : 'Installing tool' };
    }

    case 'lsp':
      return { title: 'Code intelligence' };

    case 'run_tests':
      return { title: 'Running tests' };

    case 'run_linter':
      return { title: 'Running linter' };

    case 'read_diff':
      return { title: 'Reading diff' };

    default:
      return null;
  }
}

/**
 * Recognise the small set of known integration toolkit slugs. Used to
 * gate `inferIntegrationName` so unknown `delegate_<x>` names (e.g.
 * `delegate_summarize`, `delegate_router`) don't get fake-humanised
 * into bogus "integration" labels in the tool timeline.
 */
const KNOWN_TOOLKIT_RE =
  /^(gmail|notion|github|slack|discord|linear|jira|google_calendar|google_drive|calendar)$/i;

export function inferIntegrationName(input?: string): string | undefined {
  if (!input) return undefined;

  const delegateMatch = input.match(/^delegate_(.+)$/);
  if (delegateMatch && KNOWN_TOOLKIT_RE.test(delegateMatch[1])) {
    return normalizeIntegrationName(delegateMatch[1]);
  }

  if (KNOWN_TOOLKIT_RE.test(input)) {
    return normalizeIntegrationName(input);
  }

  return undefined;
}

function integrationActivityTitle(provider: string): string {
  switch (provider) {
    case 'GitHub':
    case 'Gmail':
    case 'Linear':
    case 'Jira':
      return `Making requests to your ${provider} account`;
    case 'Notion':
      return 'Working in your Notion workspace';
    case 'Slack':
    case 'Discord':
      return `Working in your ${provider} workspace`;
    case 'Google Calendar':
      return 'Updating your Google Calendar';
    case 'Google Drive':
      return 'Working in your Google Drive';
    default:
      return `Checking your ${provider}`;
  }
}

function inferIntegrationNameFromPrompt(prompt?: string): string | undefined {
  if (!prompt) return undefined;
  const known = [
    'Notion',
    'Gmail',
    'GitHub',
    'Slack',
    'Discord',
    'Linear',
    'Jira',
    'Google Calendar',
    'Google Drive',
  ];

  const lower = prompt.toLowerCase();
  return known.find(name => lower.includes(name.toLowerCase()));
}

function parseToolArgs(argsBuffer?: string): ParsedToolArgs | null {
  if (!argsBuffer) return null;
  try {
    const parsed = JSON.parse(argsBuffer) as ParsedToolArgs;
    return parsed && typeof parsed === 'object' ? parsed : null;
  } catch {
    return null;
  }
}

function normalizeIntegrationName(value: string): string {
  switch (value.toLowerCase()) {
    case 'github':
      return 'GitHub';
    case 'gmail':
      return 'Gmail';
    case 'google_calendar':
    case 'calendar':
      return 'Google Calendar';
    case 'google_drive':
      return 'Google Drive';
    default:
      return humanizeIdentifier(value);
  }
}

function humanizeIdentifier(value: string | undefined | null): string {
  if (!value) return '';
  return value
    .replace(/^subagent:/, '')
    .replace(/^delegate_/, '')
    .replace(/_/g, ' ')
    .replace(/\b\w/g, char => char.toUpperCase());
}
