/*
 * Pure routing-map helpers for the AI settings panel.
 *
 * Kept out of AIPanel.tsx so the logic is unit-testable without rendering the
 * whole panel (the types are imported type-only, so there is no runtime import
 * cycle — AIPanel imports these functions, this module imports only erased
 * types back from it).
 */
import type { CloudProvider, ProviderRef, RoutingMap } from './AIPanel';

const LOCAL_RUNTIME_SLUGS = ['ollama', 'lmstudio'] as const;

/**
 * Reset any workload routing ref pinned to a now-removed provider back to
 * `{ kind: 'default' }` (managed), so disabling a provider can never leave
 * orphaned routing that still points at it.
 *
 * Matching differs by provider kind because routing refs carry different
 * identity:
 * - **Cloud / custom** providers are matched precisely by `providerSlug`
 *   (`{ kind: 'cloud', providerSlug, model }`).
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
    const orphanedLocal = ref.kind === 'local' && removed.isLocalRuntime && !anyLocalRuntimeLeft;
    const nextRef: ProviderRef = pinnedToRemovedCloud || orphanedLocal ? { kind: 'default' } : ref;
    return [workloadId, nextRef] as const;
  });

  return Object.fromEntries(scrubbed) as RoutingMap;
}
