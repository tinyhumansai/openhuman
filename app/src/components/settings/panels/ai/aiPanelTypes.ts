/*
 * Shared types + pure helpers for the AI settings panel family.
 *
 * Colocated here (rather than duplicated per-file) because every extracted
 * section (auth chips, workload routing, background loops) shares the same
 * `ProviderRef` / `RoutingMap` vocabulary. Kept dependency-free of React so it
 * stays trivially unit-testable.
 */
import type { ModelRegistryEntry } from '../../../../services/api/aiSettingsApi';
import type { AuthStyle } from '../../../../utils/tauriCommands/config';
import {
  authStyleForBuiltinCloudProvider,
  BUILTIN_CLOUD_PROVIDER_META,
  BUILTIN_CLOUD_PROVIDER_SLUGS,
  defaultEndpointForBuiltinCloudProvider,
} from '../builtinCloudProviders';

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

export type CloudProvider = {
  id: string;
  slug: string;
  label: string;
  endpoint: string;
  authStyle: AuthStyle;
  maskedKey: string;
};

export type OllamaState =
  | 'disabled'
  | 'missing'
  | 'stopped'
  | 'starting'
  | 'running'
  | 'degraded'
  | 'error';

export type OllamaModel = { id: string; sizeBytes: number; family: string };

export type WorkloadId =
  | 'chat'
  | 'reasoning'
  | 'agentic'
  | 'coding'
  | 'vision'
  | 'memory'
  | 'heartbeat'
  | 'learning'
  | 'subconscious';

export type WorkloadGroup = 'chat' | 'background';

export type ProviderRef =
  | { kind: 'openhuman' }
  | { kind: 'default' }
  | { kind: 'cloud'; providerSlug: string; model: string; temperature?: number | null }
  | { kind: 'local'; model: string; temperature?: number | null }
  | { kind: 'claude-code'; model: string; temperature?: number | null };

export type Workload = {
  id: WorkloadId;
  group: WorkloadGroup;
  // i18n keys (resolved with `t()` at render) rather than literal English, so the
  // workload labels/descriptions translate like the rest of the panel.
  labelKey: string;
  descriptionKey: string;
};

export type RoutingMap = Record<WorkloadId, ProviderRef>;
export type RoutingMode = 'managed' | 'own' | 'custom';

export type AISettings = {
  cloudProviders: CloudProvider[];
  routing: RoutingMap;
  modelRegistry: ModelRegistryEntry[];
};

/** Local-runtime chip slugs (Ollama / LM Studio / OMLX) that aren't actual
 *  slugs in the cloud_providers list but need the same chip affordance. */
export type LocalChipSlug = 'lmstudio' | 'ollama' | 'omlx';

export type CustomDialogSource =
  /**
   * Managed routing — OpenHuman picks the model. It carries no model id of its
   * own, which is the whole point: it is the "let the product decide" option,
   * and it exists in this union so the shared picker can offer a way BACK to
   * managed. Without it, choosing any specific model was a one-way door.
   *
   * Maps to `ProviderRef` `{ kind: 'default' }` for routing, and to a null
   * model override in the chat composer.
   */
  | { kind: 'managed' }
  | { kind: 'cloud'; providerSlug: string }
  | { kind: 'local' }
  | { kind: 'claude-code' };

/**
 * The live `/models` verification rejected. Distinguished from every other
 * submit failure (bad slug, key write failure, …) so the editor can offer to
 * add the provider without verifying: a provider that does not serve an
 * OpenAI-shaped `{base}/models` listing — Azure's classic `api-version`
 * surface, a chat-only gateway — is still usable for inference, and blocking
 * creation on the probe left those users with no way to reach the model /
 * deployment-name field at all (#5213).
 */
export class ProviderProbeError extends Error {
  readonly probeFailed = true;
}

/** Default model identifier presented when the user first picks the Claude
 * Code CLI source. This string is passed verbatim to `claude --model`, so it
 * MUST be a value the CLI accepts — an alias (`sonnet`, `opus`, `fable`) or a
 * full name (`claude-sonnet-4-5`). NOT a marketing string like `sonnet-4-5`,
 * which the CLI rejects with "model may not exist". `sonnet` tracks the latest
 * Sonnet the signed-in account can run. */
export const CLAUDE_CODE_DEFAULT_MODEL = 'sonnet';

export const ROUTING_WORKLOAD_IDS: WorkloadId[] = [
  'chat',
  'reasoning',
  'agentic',
  'coding',
  'vision',
  'memory',
  'heartbeat',
  'learning',
  'subconscious',
];

export const BUILTIN_RESERVED_SLUGS = [
  'cloud',
  'openhuman',
  'pid',
  'custom',
  'ollama',
  'lmstudio',
  'omlx',
  // Claude Code is a CLI-backed peer provider surfaced via a dedicated
  // connect button (not a chip), so reserve its slug so it never renders in
  // the generic custom-provider chip list.
  'claude-code',
  ...BUILTIN_CLOUD_PROVIDER_SLUGS,
];

export const KIMI_PLATFORM_URL = 'https://platform.kimi.ai?aff=openhuman';

// Slug-keyed display metadata for built-in provider slugs. Used only for
// chip rendering (label, tone). Custom providers use `provider.label` directly.
export const BUILTIN_PROVIDER_META: Record<string, { tone: string; label: string }> = {
  openhuman: {
    label: 'Managed',
    tone: 'border-sage-200 bg-sage-50 ring-sage-200 text-sage-900 dark:bg-sage-500/10 dark:text-sage-100',
  },
  ...BUILTIN_CLOUD_PROVIDER_META,
  custom: {
    label: 'Advanced',
    tone: 'border-primary-200 bg-primary-50 ring-primary-200 text-primary-900 dark:bg-primary-500/10 dark:text-primary-100',
  },
};

// Tints per local-runtime chip slug — kept to the semantic palette (primary /
// amber / neutral) rather than decorative stock Tailwind hues.
export const LOCAL_CHIP_TONE: Record<LocalChipSlug, string> = {
  lmstudio:
    'bg-primary-50 dark:bg-primary-500/10 ring-primary-200 text-primary-900 dark:text-primary-100',
  ollama: 'bg-surface-subtle ring-line-strong text-content',
  omlx: 'bg-amber-50 dark:bg-amber-500/10 ring-amber-200 text-amber-900 dark:text-amber-100',
};

export const LOCAL_CHIP_LABEL: Record<LocalChipSlug, string> = {
  lmstudio: 'LM Studio',
  ollama: 'Ollama',
  omlx: 'OMLX',
};

export const WORKLOADS: Workload[] = [
  {
    id: 'chat',
    group: 'chat',
    labelKey: 'settings.ai.routing.workload.chat.label',
    descriptionKey: 'settings.ai.routing.workload.chat.description',
  },
  {
    id: 'reasoning',
    group: 'chat',
    labelKey: 'settings.ai.routing.workload.reasoning.label',
    descriptionKey: 'settings.ai.routing.workload.reasoning.description',
  },
  {
    id: 'agentic',
    group: 'chat',
    labelKey: 'settings.ai.routing.workload.agentic.label',
    descriptionKey: 'settings.ai.routing.workload.agentic.description',
  },
  {
    id: 'coding',
    group: 'chat',
    labelKey: 'settings.ai.routing.workload.coding.label',
    descriptionKey: 'settings.ai.routing.workload.coding.description',
  },
  {
    id: 'vision',
    group: 'chat',
    labelKey: 'settings.ai.routing.workload.vision.label',
    descriptionKey: 'settings.ai.routing.workload.vision.description',
  },
  {
    id: 'memory',
    group: 'background',
    labelKey: 'settings.ai.routing.workload.memory.label',
    descriptionKey: 'settings.ai.routing.workload.memory.description',
  },
  {
    id: 'heartbeat',
    group: 'background',
    labelKey: 'settings.ai.routing.workload.heartbeat.label',
    descriptionKey: 'settings.ai.routing.workload.heartbeat.description',
  },
  {
    id: 'learning',
    group: 'background',
    labelKey: 'settings.ai.routing.workload.learning.label',
    descriptionKey: 'settings.ai.routing.workload.learning.description',
  },
  {
    id: 'subconscious',
    group: 'background',
    labelKey: 'settings.ai.routing.workload.subconscious.label',
    descriptionKey: 'settings.ai.routing.workload.subconscious.description',
  },
];

// i18n keys for the per-workload "Recommended: …" hints (resolved with `t()`).
export const WORKLOAD_MODEL_HINT_KEYS: Record<WorkloadId, string> = {
  chat: 'settings.ai.routing.workload.chat.hint',
  reasoning: 'settings.ai.routing.workload.reasoning.hint',
  agentic: 'settings.ai.routing.workload.agentic.hint',
  coding: 'settings.ai.routing.workload.coding.hint',
  vision: 'settings.ai.routing.workload.vision.hint',
  memory: 'settings.ai.routing.workload.memory.hint',
  heartbeat: 'settings.ai.routing.workload.heartbeat.hint',
  learning: 'settings.ai.routing.workload.learning.hint',
  subconscious: 'settings.ai.routing.workload.subconscious.hint',
};

export const EMPTY_ROUTING: RoutingMap = {
  chat: { kind: 'default' },
  reasoning: { kind: 'default' },
  agentic: { kind: 'default' },
  coding: { kind: 'default' },
  vision: { kind: 'default' },
  memory: { kind: 'default' },
  heartbeat: { kind: 'default' },
  learning: { kind: 'default' },
  subconscious: { kind: 'default' },
};

export const EMPTY_SETTINGS: AISettings = {
  cloudProviders: [],
  routing: EMPTY_ROUTING,
  modelRegistry: [],
};

// ─────────────────────────────────────────────────────────────────────────────
// Pure helpers
// ─────────────────────────────────────────────────────────────────────────────

// Build the "pending routing changes" summary: one `"<label> → <target>"`
// entry per workload whose draft route differs from the saved route. A pure,
// exported function so the (translated) formatting is unit-testable without
// rendering the whole panel.
export function buildRoutingDiffSummary(
  saved: RoutingMap,
  draft: RoutingMap,
  t: (key: string) => string
): string[] {
  const describe = (r: ProviderRef): string => {
    if (r.kind === 'openhuman') return 'openhuman';
    if (r.kind === 'default') return 'cloud';
    const tempSuffix = r.temperature != null ? `@${r.temperature.toFixed(2)}` : '';
    if (r.kind === 'cloud') return `${r.providerSlug}:${r.model}${tempSuffix}`;
    return `local:${r.model}${tempSuffix}`;
  };
  const out: string[] = [];
  for (const w of WORKLOADS) {
    const a = saved[w.id];
    const b = draft[w.id];
    if (JSON.stringify(a) !== JSON.stringify(b)) {
      out.push(`${t(w.labelKey)} → ${describe(b)}`);
    }
  }
  return out;
}

export function maskKeyLabel(hasKey: boolean): string {
  return hasKey ? '•••• configured' : 'Not configured';
}

export function slugifyCustomProviderName(name: string): string {
  return name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
}

/**
 * Default auth style for a slug. Built-in slugs map to their known styles;
 * everything else (custom + third-party slugs the user types in) defaults
 * to bearer, matching the OpenAI-compatible majority.
 */
export function authStyleForSlug(slug: string): AuthStyle {
  if (slug === 'openhuman') return 'openhuman_jwt';
  if (slug === 'lmstudio' || slug === 'ollama') return 'none';
  if (slug === 'omlx') return 'bearer';
  // Claude Code authenticates via the local CLI, never an HTTP key.
  if (slug === 'claude-code') return 'none';
  return authStyleForBuiltinCloudProvider(slug) ?? 'bearer';
}

export function formatI18n(template: string, vars: Record<string, string | number>): string {
  return Object.entries(vars).reduce(
    (result, [key, value]) => result.replaceAll(`{${key}}`, String(value)),
    template
  );
}

export function slugTone(slug: string): string {
  return BUILTIN_PROVIDER_META[slug]?.tone ?? 'bg-surface-subtle ring-line-strong text-content';
}

export function providerToggleAriaLabel(
  t: (key: string, fallback?: string) => string,
  enabled: boolean,
  label: string
): string {
  return formatI18n(
    enabled ? t('settings.ai.disconnectProvider') : t('settings.ai.connectProviderLabel'),
    { label }
  );
}

export function appendTemperatureToProviderString(
  provider: string,
  temperature: number | null
): string {
  if (temperature == null || !Number.isFinite(temperature)) return provider;
  const rounded = Math.round(temperature * 100) / 100;
  return `${provider}@${String(rounded)}`;
}

export function providerRefSignature(ref: ProviderRef): string {
  switch (ref.kind) {
    case 'openhuman':
      return 'openhuman';
    case 'default':
      return 'default';
    case 'cloud':
      return `cloud:${ref.providerSlug}:${ref.model}:${ref.temperature ?? ''}`;
    case 'local':
      return `local:${ref.model}:${ref.temperature ?? ''}`;
    case 'claude-code':
      return `claude-code:${ref.model}:${ref.temperature ?? ''}`;
  }
}

export function inferRoutingMode(routing: RoutingMap): RoutingMode {
  const refs = ROUTING_WORKLOAD_IDS.map(id => routing[id]);
  if (refs.every(ref => ref.kind === 'openhuman' || ref.kind === 'default')) {
    return 'managed';
  }
  const first = refs[0];
  if (
    first &&
    (first.kind === 'cloud' || first.kind === 'local') &&
    refs.every(ref => providerRefSignature(ref) === providerRefSignature(first))
  ) {
    return 'own';
  }
  return 'custom';
}

export function inferSharedModelRef(routing: RoutingMap): ProviderRef | null {
  const refs = ROUTING_WORKLOAD_IDS.map(id => routing[id]);
  const first = refs[0];
  if (!first) return null;
  if (refs.every(ref => providerRefSignature(ref) === providerRefSignature(first))) {
    return first.kind === 'openhuman' ? null : first;
  }
  return (
    refs.find(ref => ref.kind === 'cloud' || ref.kind === 'local' || ref.kind === 'default') ?? null
  );
}

export function routingWithAllWorkloads(next: ProviderRef): RoutingMap {
  return {
    chat: next,
    reasoning: next,
    agentic: next,
    coding: next,
    vision: next,
    memory: next,
    heartbeat: next,
    learning: next,
    subconscious: next,
  };
}

export function defaultEndpointFor(slug: string): string {
  const builtinEndpoint = defaultEndpointForBuiltinCloudProvider(slug);
  if (builtinEndpoint) return builtinEndpoint;

  switch (slug) {
    case 'openhuman':
      return 'https://api.openhuman.ai/v1';
    // Cosmetic only — the claude-code factory branch never makes HTTP calls.
    case 'claude-code':
      return 'cli://claude-code';
    case 'ollama':
      // Ollama exposes an OpenAI-compatible endpoint at /v1; the bare host is
      // also accepted by the Rust factory (it appends /v1 internally for chat).
      // For the /models probe we want the OpenAI-compat path.
      return 'http://localhost:11434/v1';
    case 'lmstudio':
      return 'http://localhost:1234/v1';
    case 'omlx':
      return 'http://localhost:8000/v1';
    default:
      return '';
  }
}
