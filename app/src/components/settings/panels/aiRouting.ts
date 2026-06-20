/*
 * Pure routing-map helpers for the AI settings panel.
 *
 * Kept out of AIPanel.tsx so the logic is unit-testable without rendering the
 * whole panel (the types are imported type-only, so there is no runtime import
 * cycle — AIPanel imports these functions, this module imports only erased
 * types back from it).
 */
import type { CloudProvider, ProviderRef, RoutingMap } from './AIPanel';

const LOCAL_RUNTIME_SLUGS = ['ollama', 'lmstudio', 'omlx'] as const;

/**
 * Reset any workload routing ref pinned to a now-removed provider back to
 * `{ kind: 'default' }` (managed), so disabling a provider can never leave
 * orphaned routing that still points at it.
 *
 * Matching differs by provider kind because routing refs carry different
 * identity:
 * - **Cloud / custom** providers are matched precisely by `providerSlug`
 *   (`{ kind: 'cloud', providerSlug, model }`).
 * - **Claude Code** refs carry no `providerSlug` (`{ kind: 'claude-code', model }`),
 *   so they are matched by the well-known `claude-code` slug. Without this,
 *   disconnecting Claude Code left workloads pinned to `claude-code:<model>`,
 *   which the Rust factory still honors — so chats kept using the CLI after
 *   the provider entry was removed.
 * - **Local runtimes** (Ollama / LM Studio) have NO slug on their routing refs
 *   (`{ kind: 'local', model }`), so an individual local ref can't be tied back
 *   to a specific runtime. A `local` ref is therefore only definitively
 *   orphaned once NO local runtime remains enabled; while another local runtime
 *   is still enabled we leave `local` refs alone since they may resolve to the
 *   survivor.
 *
 * Before this helper the local case was silently a no-op: the toggle-off
 * handlers only matched `kind === 'cloud' && providerSlug === <runtime>`, which
 * a `kind: 'local'` ref can never satisfy — so disabling Ollama / LM Studio left
 * its routed workloads pinned to a now-removed runtime.
 */
export function routingWithProviderRemoved(
  routing: RoutingMap,
  removed: { slug: string; isLocalRuntime: boolean },
  remainingProviders: readonly CloudProvider[]
): RoutingMap {
  const anyLocalRuntimeLeft = remainingProviders.some(p =>
    (LOCAL_RUNTIME_SLUGS as readonly string[]).includes(p.slug)
  );

  const scrubbed = Object.entries(routing).map(([workloadId, ref]) => {
    const pinnedToRemovedCloud = ref.kind === 'cloud' && ref.providerSlug === removed.slug;
    const pinnedToRemovedClaudeCode = ref.kind === 'claude-code' && removed.slug === 'claude-code';
    const orphanedLocal = ref.kind === 'local' && removed.isLocalRuntime && !anyLocalRuntimeLeft;
    const nextRef: ProviderRef =
      pinnedToRemovedCloud || pinnedToRemovedClaudeCode || orphanedLocal
        ? { kind: 'default' }
        : ref;
    return [workloadId, nextRef] as const;
  });

  return Object.fromEntries(scrubbed) as RoutingMap;
}

/**
 * Whether a locally-installed Ollama model may be offered as a chat/LLM model
 * in the settings pickers.
 *
 * Embedding-only models (e.g. `bge-m3`) cannot serve chat — Ollama 400s every
 * turn with `"<model>" does not support chat`, which flooded Sentry
 * (TAURI-RUST-4P6). The core reports `chat_capable: false` only when it is
 * confident a model is embedding-only (from `/api/show` `capabilities`); a
 * `null`/`undefined` value means unknown (older Ollama, or an `/api/show`
 * miss) and stays selectable — fail-open, never hide a usable model. The
 * embedding model is configured in a separate panel, so hiding these here
 * never blocks embedding selection.
 */
export function isChatSelectableLocalModel(model: { chat_capable?: boolean | null }): boolean {
  return model.chat_capable !== false;
}

/** A locally-installed model mapped to the picker shape consumed by the LLM/chat selectors. */
export interface SelectableChatModel {
  id: string;
  sizeBytes: number;
  family: string;
}

/**
 * Filter the installed-model list down to chat-selectable models (hiding
 * embedding-only ones via {@link isChatSelectableLocalModel}) and map each to
 * the `{ id, sizeBytes, family }` picker shape.
 *
 * Pure so the filter + map wiring is unit-testable without rendering the panel
 * — both picker consumers (`CustomRoutingDialog` + `GlobalOwnModelSelector`)
 * route a chat model, never the embedder (configured separately in
 * `EmbeddingsPanel`). Selecting an embedding model as chat 400s every turn on
 * Ollama (TAURI-RUST-4P6).
 */
export function toSelectableChatModels(
  installed: readonly { name: string; size?: number | null; chat_capable?: boolean | null }[]
): SelectableChatModel[] {
  return installed
    .filter(isChatSelectableLocalModel)
    .map(m => ({
      id: m.name,
      sizeBytes: m.size ?? 0,
      family: m.name.split(/[:/]/, 1)[0] ?? 'model',
    }));
}
