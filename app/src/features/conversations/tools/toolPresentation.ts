/**
 * The single answer to "how do we show this tool call?".
 *
 * Before this module four systems labelled tool calls and disagreed: a name
 * table, an args-sniffing heuristic that called anything with a `query`
 * argument "Searched the web", a category icon table, and the core's
 * humanized name. Every surface (chat card, timeline, processing panel,
 * status line, mascot) now resolves through {@link describeToolCall}.
 *
 * Resolution order, first hit wins:
 *
 * 1. `tool_call { name, arguments }`: the harness's deferred-tool bridge is
 *    described as the tool it invokes.
 * 2. Named agents: `subagent:<id>`, `spawn_subagent { agent_id }`,
 *    `delegate_<id>` and custom delegate names.
 * 3. Collapsed tools switching on an argument (`memory { action }`).
 * 4. An exact entry in `toolSpecs.ts`.
 * 5. A prefix family rule.
 * 6. A Composio action slug (`GMAIL_SEND_EMAIL` → "Used Gmail · Send email").
 * 7. The server's display label, for dynamic tools the client cannot know.
 * 8. A sentence-cased fallback ("Used Frobnicate widget"). Never raw
 *    snake_case, never ALL CAPS.
 *
 * Pure and synchronous: safe in reducers, selectors and tests. Translation
 * happens at the edge through {@link toolLabel} with the caller's `t`.
 */
import type { LucideIcon } from 'lucide-react';

import { matchComposioActionSlug } from '../../../components/composio/toolkitMeta';
import { chip as chipRules, genericChip, type ToolArgs, truncateChip } from './toolChips';
import {
  fillPlaceholders,
  phraseKey,
  TOOL_PHRASES,
  type ToolPhraseId,
  type ToolPhraseTense,
} from './toolPhrases';
import {
  ACTION_TOOL_SPECS,
  AGENT_SPECS,
  EXACT_TOOL_SPECS,
  FALLBACK_ICON,
  FAMILY_TOOL_SPECS,
  INTEGRATION_ICON,
  INTEGRATIONS_AGENT_ID,
  type ToolBodyKind,
  type ToolCategory,
  type ToolSpec,
} from './toolSpecs';

export type { ToolBodyKind, ToolCategory } from './toolSpecs';

/** Mirrors `ToolTimelineEntryStatus`; kept local so this module has no store import. */
export type ToolCallStatus = 'running' | 'success' | 'error' | 'awaiting_user' | 'cancelled';

/** How a presentation was resolved. `fallback` is what the catalog test forbids. */
export type ToolPresentationSource =
  | 'exact'
  | 'action'
  | 'family'
  | 'agent'
  | 'integration'
  | 'server'
  | 'fallback';

export interface DescribeToolCallInput {
  /** Tool name as streamed; may carry a `subagent:` prefix. */
  name: string;
  /** Parsed args object, or the raw JSON args buffer. */
  args?: unknown;
  status?: ToolCallStatus;
  /** `tool_display_label` from the core, for tools the client cannot know. */
  serverLabel?: string;
  /** `tool_display_detail` from the core. */
  serverDetail?: string;
  /**
   * Connected-app slug known from context rather than args: a spawned
   * `integrations_agent` row carries the `delegate_<toolkit>` tool that
   * spawned it.
   */
  toolkitHint?: string;
}

export interface ToolCallPresentation {
  /** Name with any `subagent:` prefix removed. */
  baseName: string;
  icon: LucideIcon;
  category: ToolCategory;
  body: ToolBodyKind;
  tense: ToolPhraseTense;
  /** Translatable phrase. Absent only when {@link literal} carries the label. */
  phrase?: ToolPhraseId;
  params?: Record<string, string>;
  /** Untranslatable label (a server-supplied one). */
  literal?: string;
  /** Short target beside the label: a path, query, host or app action. */
  chip?: string;
  /** Connected app, for Composio actions and the integrations agent. */
  integration?: { slug: string; name: string; known: boolean };
  source: ToolPresentationSource;
}

export type Translate = (key: string, fallback?: string) => string;

const ACTIVE_STATUSES = new Set<ToolCallStatus>(['running', 'awaiting_user']);

export function tenseForStatus(status: ToolCallStatus | undefined): ToolPhraseTense {
  return !status || ACTIVE_STATUSES.has(status) ? 'active' : 'done';
}

/** Parse args from an object or a JSON buffer; anything else is `{}`. */
export function parseToolArgs(args: unknown): ToolArgs {
  if (args && typeof args === 'object' && !Array.isArray(args)) return args as ToolArgs;
  if (typeof args !== 'string' || !args.trim()) return {};
  try {
    const parsed: unknown = JSON.parse(args);
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed)
      ? (parsed as ToolArgs)
      : {};
  } catch {
    return {};
  }
}

/**
 * `web_search_tool` → "Web search tool", `GMAIL_SEND_EMAIL` → "Gmail send
 * email", `fooBar` → "Foo bar". Lower-cases everything after the first
 * letter so a slug never renders shouting.
 */
export function sentenceCase(value: string): string {
  const words = value
    .replace(/^subagent:/, '')
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .replace(/[_\-.:/]+/g, ' ')
    .trim()
    .toLowerCase();
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/** Is a server label readable as-is, or is it a leaked identifier? */
function isReadableLabel(label: string, rawName: string): boolean {
  const trimmed = label.trim();
  if (!trimmed || trimmed.toLowerCase() === 'tool') return false;
  if (trimmed === rawName) return false;
  if (/[_]/.test(trimmed)) return false;
  // Two or more consecutive ALL-CAPS words ("GMAIL SEND EMAIL").
  if (/\b[A-Z]{2,}\b\s+\b[A-Z]{2,}\b/.test(trimmed)) return false;
  return true;
}

function fromSpec(
  spec: ToolSpec,
  source: ToolPresentationSource,
  baseName: string,
  args: ToolArgs,
  tense: ToolPhraseTense,
  serverDetail: string | undefined
): ToolCallPresentation {
  return {
    baseName,
    icon: spec.icon,
    category: spec.category,
    body: spec.body ?? 'generic',
    tense,
    phrase: spec.phrase,
    chip: spec.chip?.(args) ?? cleanDetail(serverDetail),
    source,
  };
}

function cleanDetail(detail: string | undefined): string | undefined {
  return detail?.trim() ? truncateChip(detail) : undefined;
}

function agentPresentation(
  agentId: string,
  baseName: string,
  args: ToolArgs,
  tense: ToolPhraseTense,
  serverDetail: string | undefined,
  toolkitHint?: string
): ToolCallPresentation | undefined {
  if (agentId === INTEGRATIONS_AGENT_ID) {
    const toolkit = typeof args.toolkit === 'string' ? args.toolkit : toolkitHint;
    const app = toolkit ? integrationFromToolkit(toolkit) : undefined;
    const prompt = typeof args.prompt === 'string' ? args.prompt : serverDetail;
    return {
      baseName,
      icon: INTEGRATION_ICON,
      category: 'app',
      body: 'generic',
      tense,
      ...(app
        ? { phrase: 'useApp' as const, params: { app: app.name }, integration: app }
        : { phrase: 'checkConnectedApp' as const }),
      chip: cleanDetail(prompt),
      source: 'agent',
    };
  }
  const spec = AGENT_SPECS[agentId];
  if (!spec) return undefined;
  const prompt = typeof args.prompt === 'string' ? args.prompt : undefined;
  return {
    ...fromSpec(spec, 'agent', baseName, args, tense, serverDetail),
    chip: cleanDetail(serverDetail) ?? cleanDetail(prompt),
  };
}

function integrationFromToolkit(
  toolkit: string
): { slug: string; name: string; known: boolean } | undefined {
  const slug = toolkit.trim().toLowerCase();
  if (!slug) return undefined;
  // Reuse the action matcher's catalog lookup with a synthetic action.
  const match = matchComposioActionSlug(`${slug.toUpperCase()}_X`);
  return match ? { slug: match.slug, name: match.name, known: match.known } : undefined;
}

export function describeToolCall(input: DescribeToolCallInput): ToolCallPresentation {
  const rawName = input.name?.trim() || 'tool';
  const baseName = rawName.replace(/^subagent:/, '');
  const args = parseToolArgs(input.args);
  const tense = tenseForStatus(input.status);
  const { serverDetail } = input;

  // 1. Deferred-tool bridge: describe the tool it actually calls.
  if (baseName === 'tool_call' && typeof args.name === 'string' && args.name.trim()) {
    return describeToolCall({
      ...input,
      name: args.name,
      args: args.arguments,
      serverLabel: undefined,
    });
  }

  // 2. Named agents.
  if (rawName.startsWith('subagent:') || baseName === INTEGRATIONS_AGENT_ID) {
    const hint = input.toolkitHint?.replace(/^delegate_/, '');
    const agent = agentPresentation(baseName, baseName, args, tense, serverDetail, hint);
    if (agent) return agent;
  }
  if (
    (baseName === 'spawn_subagent' || baseName === 'spawn_async_subagent') &&
    typeof args.agent_id === 'string'
  ) {
    const agent = agentPresentation(args.agent_id, baseName, args, tense, serverDetail);
    if (agent) return agent;
  }
  if (baseName.startsWith('delegate_') && !EXACT_TOOL_SPECS[baseName]) {
    const id = baseName.slice('delegate_'.length);
    // An app named in the args (`delegate_tools_agent { toolkit: "github" }`)
    // says more than the generic agent does, so it wins over the agent spec.
    const argApp =
      typeof args.toolkit === 'string' ? integrationFromToolkit(args.toolkit) : undefined;
    const app = argApp?.known ? argApp : integrationFromToolkit(id);
    const agent = argApp?.known
      ? undefined
      : agentPresentation(id, baseName, args, tense, serverDetail);
    if (agent) return agent;
    if (app?.known) {
      return {
        baseName,
        icon: INTEGRATION_ICON,
        category: 'app',
        body: 'generic',
        tense,
        phrase: 'useApp',
        params: { app: app.name },
        integration: app,
        chip: cleanDetail(typeof args.prompt === 'string' ? args.prompt : serverDetail),
        source: 'agent',
      };
    }
    return {
      baseName,
      // Delegate aliases are no longer registered but remain in older
      // transcripts, so give them a stable agent icon without a live spec.
      icon: EXACT_TOOL_SPECS.spawn_subagent.icon,
      category: 'agent',
      body: 'generic',
      tense,
      phrase: 'delegateTask',
      chip: sentenceCase(id),
      source: 'agent',
    };
  }
  if (AGENT_SPECS[baseName] && !EXACT_TOOL_SPECS[baseName]) {
    const agent = agentPresentation(baseName, baseName, args, tense, serverDetail);
    if (agent) return agent;
  }

  // 3. Collapsed tools that switch on an argument.
  const action = ACTION_TOOL_SPECS[baseName];
  if (action) {
    const value = args[action.arg];
    const actionSpec = typeof value === 'string' ? action.specs[value] : undefined;
    if (actionSpec) return fromSpec(actionSpec, 'action', baseName, args, tense, serverDetail);
  }

  // 4. Exact entry.
  const exact = EXACT_TOOL_SPECS[baseName];
  if (exact) {
    const presentation = fromSpec(exact, 'exact', baseName, args, tense, serverDetail);
    if (baseName === 'mcp_call_tool' || baseName === 'mcp_registry_tool_call') {
      const tool =
        typeof args.tool === 'string'
          ? args.tool
          : typeof args.tool_name === 'string'
            ? args.tool_name
            : undefined;
      if (tool?.trim()) presentation.params = { tool: truncateChip(tool, 48) };
      else presentation.phrase = 'checkMcpTools';
    }
    if (baseName === 'composio_execute' && typeof args.tool === 'string') {
      const match = matchComposioActionSlug(args.tool);
      if (match) {
        return {
          ...presentation,
          phrase: 'useApp',
          params: { app: match.name },
          integration: { slug: match.slug, name: match.name, known: match.known },
          chip: match.action,
        };
      }
    }
    return presentation;
  }

  // 5. Prefix family.
  const family = FAMILY_TOOL_SPECS.find(rule => rule.test.test(baseName));
  if (family) return fromSpec(family.spec, 'family', baseName, args, tense, serverDetail);

  // 6. Composio action slug.
  const composio = matchComposioActionSlug(baseName);
  if (composio) {
    // The action names what was done; the server detail (or the obvious
    // argument, e.g. a recipient) names what it was done to.
    const target = cleanDetail(serverDetail) ?? genericChip(args);
    return {
      baseName,
      icon: INTEGRATION_ICON,
      category: 'app',
      body: 'generic',
      tense,
      phrase: 'useApp',
      params: { app: composio.name },
      integration: { slug: composio.slug, name: composio.name, known: composio.known },
      chip: target ? truncateChip(`${composio.action} · ${target}`) : composio.action,
      source: 'integration',
    };
  }

  // 7. Server label for a dynamic tool.
  const serverLabel = input.serverLabel?.trim();
  if (serverLabel && isReadableLabel(serverLabel, rawName)) {
    return {
      baseName,
      icon: FALLBACK_ICON,
      category: 'other',
      body: 'generic',
      tense,
      literal: serverLabel,
      chip: cleanDetail(serverDetail) ?? genericChip(args),
      source: 'server',
    };
  }

  // 8. Fallback.
  return {
    baseName,
    icon: FALLBACK_ICON,
    category: 'other',
    body: 'generic',
    tense,
    phrase: 'useTool',
    params: { tool: sentenceCase(baseName).toLowerCase() },
    chip: cleanDetail(serverDetail) ?? genericChip(args),
    source: 'fallback',
  };
}

/** English label, for logs and non-React callers. */
export function toolLabel(presentation: ToolCallPresentation, t?: Translate): string {
  if (presentation.literal) return presentation.literal;
  const id = presentation.phrase ?? 'useTool';
  const english = TOOL_PHRASES[id][presentation.tense];
  const template = t ? t(phraseKey(id, presentation.tense), english) : english;
  const label = fillPlaceholders(template, presentation.params);
  // `useTool` with a lower-cased tool name reads "Using frobnicate"; lift the
  // first letter of the whole label only.
  return label.charAt(0).toUpperCase() + label.slice(1);
}

/** Every chip rule, exported for the gallery and tests. */
export const TOOL_CHIP_RULES = chipRules;
