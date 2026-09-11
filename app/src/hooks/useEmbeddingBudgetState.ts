/**
 * Memory-embedding budget state (#5324).
 *
 * The failure this exists to prevent: a heavy user's managed embedding budget
 * runs out, every embed job fails as `unrecoverable`, and the Memory Tree
 * silently stops growing. The only signal was a yellow banner inside a
 * settings panel nobody opens, so users experienced it as "the app has been
 * broken for a month" without knowing why.
 *
 * ## Which budget this reads, and why
 *
 * There is no separate embedding meter. Managed embeddings are billed against
 * the *same* managed cycle budget as chat — the cloud embed route returns the
 * identical `USER_INSUFFICIENT_CREDITS` / "Insufficient budget" error — so
 * `useUsageState().usagePct` is the authoritative consumption figure for both.
 * This hook adds the memory-specific *framing* on top: it only fires when the
 * user's embeddings actually route through that managed budget, and it steers
 * toward the embedding-specific fixes (local Ollama, BYO key) rather than the
 * plan upgrade `GlobalUpsellBanner` already offers.
 *
 * A user on local Ollama or a BYO key is unaffected by the managed budget for
 * embeddings, so they must never see this — a false alarm here would teach
 * users to ignore the real one.
 */
import { useCallback, useEffect, useState } from 'react';

import { useCoreState } from '../providers/CoreStateProvider';
import { creditsApi, type TeamUsage } from '../services/api/creditsApi';
import { loadEmbeddingsSettings } from '../services/api/embeddingsApi';
import { CoreRpcError } from '../services/coreRpcClient';
import { subscribeUsageRefresh } from './usageRefresh';
import { useUsageState } from './useUsageState';

/** Consumption at which the dismissible early warning appears. */
export const EMBEDDING_BUDGET_WARN_PCT = 0.75;
/** Consumption at which the warning becomes non-dismissible. */
export const EMBEDDING_BUDGET_URGENT_PCT = 0.9;

/**
 * Provider slugs that bill against the managed cycle budget. Everything else
 * (`ollama:*`, `openai`, `voyage`, `custom:*`, `none`, `unconfigured`,
 * `unknown`, …) is funded by the user — or by nobody — so the managed budget
 * running out does not stop their memory from growing.
 */
const MANAGED_PROVIDER_SLUGS = ['openhuman', 'managed', 'cloud'];

/** Grep prefix for this hook's lifecycle diagnostics. */
const LOG = '[embedding-budget]';

/**
 * Privacy-safe error label. Never log the raw error: it can carry endpoint
 * URLs, request bodies, or backend messages quoting user content. A kind plus
 * (for our own typed errors) the stable discriminator is enough to diagnose.
 */
function errorKind(err: unknown): string {
  if (err instanceof CoreRpcError) return `core_rpc:${err.kind}`;
  if (err instanceof Error) return `error:${err.name}`;
  return 'unknown';
}

export type EmbeddingBudgetLevel = 'none' | 'warn' | 'urgent' | 'exhausted';

export interface EmbeddingBudgetState {
  /** Which banner (if any) the user should see. `none` renders nothing. */
  level: EmbeddingBudgetLevel;
  /** Whole-percent consumption, for the warning copy. */
  pct: number;
  /** True while the provider or usage read is still in flight. */
  isLoading: boolean;
  /** True when embeddings bill against the managed budget. */
  isManagedEmbeddings: boolean;
}

/** True when `provider` bills against the managed cycle budget. */
export function isManagedEmbeddingProvider(provider: string | null | undefined): boolean {
  if (!provider) return false;
  // Providers are stored either bare (`openhuman`) or as `slug:model`
  // (`ollama:nomic-embed-text`), so compare on the slug only.
  const slug = provider.trim().toLowerCase().split(':')[0];
  return MANAGED_PROVIDER_SLUGS.includes(slug);
}

/**
 * Pure threshold mapping, exported so the levels can be tested without
 * mocking the RPC layer.
 *
 * `isExhausted` wins over the percentage because the two can disagree: a
 * hard `remainingUsd <= 0` verdict is authoritative even if the derived
 * percentage rounds to something under 100.
 */
export function embeddingBudgetLevel(usagePct: number, isExhausted: boolean): EmbeddingBudgetLevel {
  if (isExhausted) return 'exhausted';
  if (usagePct >= EMBEDDING_BUDGET_URGENT_PCT) return 'urgent';
  if (usagePct >= EMBEDDING_BUDGET_WARN_PCT) return 'warn';
  return 'none';
}

/** Derived budget snapshot: percent consumed + hard-exhausted verdict. */
interface DerivedBudget {
  pct: number;
  exhausted: boolean;
}

/**
 * Percent-consumed + exhausted verdict from a raw `TeamUsage`. Byte-identical
 * to `useUsageState`'s own derivation so the fallback path (below) can never
 * disagree with the primary path on the same underlying budget.
 */
function deriveBudget(usage: TeamUsage): DerivedBudget {
  const pct =
    usage.cycleBudgetUsd > 0.01
      ? Math.max(0, Math.min(1, (usage.cycleBudgetUsd - usage.remainingUsd) / usage.cycleBudgetUsd))
      : 0;
  const exhausted = usage.cycleBudgetUsd > 0.01 && usage.remainingUsd <= 0.01;
  return { pct, exhausted };
}

/**
 * How often the embeddings provider is re-read while it still bills against
 * the managed budget. Matches `useUsageState`'s cache TTL.
 */
const PROVIDER_RECHECK_MS = 60_000;

export function useEmbeddingBudgetState(): EmbeddingBudgetState {
  const { snapshot } = useCoreState();
  const isAuthenticated = snapshot.auth.isAuthenticated;
  const { usagePct, isBudgetExhausted, isLoading: usageLoading, teamUsage } = useUsageState();
  const [provider, setProvider] = useState<string | null>(null);
  const [providerLoading, setProviderLoading] = useState(true);
  // Fallback budget snapshot for the routed-away-but-managed-embeddings case.
  // `useUsageState` deliberately returns no `teamUsage` when every chat +
  // background workload is routed off OpenHuman (#2020 privacy optimisation) —
  // but managed embeddings still bill against the managed cycle budget, so we
  // read it directly there rather than letting that bypass silence the warning.
  const [fallbackUsage, setFallbackUsage] = useState<DerivedBudget | null>(null);
  const [reloadCount, setReloadCount] = useState(0);

  const reload = useCallback(() => setReloadCount(n => n + 1), []);

  // Depend on the *presence* of a usage payload, not the object itself — the
  // object identity is not part of this hook's contract, and keying an effect
  // on it would re-fire the read on every render for any caller whose
  // `useUsageState` returns a fresh object.
  const hasUsage = teamUsage !== null;

  useEffect(() => {
    // Gate the provider/budget reads on a live session, NOT on the presence of
    // a usage payload. `teamUsage` is null both when signed out AND when an
    // authenticated user has routed chat away while keeping managed embeddings
    // — the two must be handled differently, so `hasUsage` cannot be the gate.
    //
    // Signed out / offline: clear any provider carried over from a previous
    // user so the next session cannot combine this user's usage with the prior
    // user's managed provider (which would show a false managed-budget warning
    // before the fresh read resolves). Then skip the RPCs, which require a
    // session anyway.
    if (!isAuthenticated) {
      console.debug(`${LOG} skip: not authenticated — cleared provider + fallback budget`);
      setProvider(null);
      setFallbackUsage(null);
      setProviderLoading(false);
      return;
    }
    let cancelled = false;
    setProviderLoading(true);
    console.debug(`${LOG} provider read start (hasUsage=${hasUsage} usageLoading=${usageLoading})`);
    void (async () => {
      try {
        const settings = await loadEmbeddingsSettings();
        if (cancelled) return;
        // Gate on the *effective* embedder, not the picker setting. The core
        // resolves local Ollama from the Local AI "Memory embeddings" toggle or
        // the `memory_tree.embedding_endpoint` override, and neither rewrites
        // `provider` — so a fully-local user still reads `provider: "cloud"`
        // there and would be told their memory stopped growing while it is
        // growing fine (reviewer M3gA-Mind, #5402). Fall back to `provider`
        // only for a core old enough not to send the field.
        const nextProvider = settings.effective_provider ?? settings.provider;
        const managed = isManagedEmbeddingProvider(nextProvider);
        console.debug(
          `${LOG} provider read ok: effective=${nextProvider} ` +
            `configured=${settings.provider} managed=${managed}`
        );
        setProvider(nextProvider);
        // Only reach for the direct budget read when it is actually needed:
        // embeddings bill against the managed budget AND `useUsageState` has
        // SETTLED with no payload (chat routed away). `teamUsage` is also null
        // while `useUsageState`'s own request is still in flight, so gate on
        // `!usageLoading` too — otherwise a normal managed user whose provider
        // read resolves first fires a redundant `getTeamUsage()` that
        // `useUsageState` is about to make anyway.
        if (managed && !hasUsage && !usageLoading) {
          console.debug(`${LOG} fallback budget read start (managed + usage settled empty)`);
          try {
            const usage = await creditsApi.getTeamUsage();
            if (!cancelled) setFallbackUsage(deriveBudget(usage));
            console.debug(`${LOG} fallback budget read ok`);
          } catch (err) {
            // Auth-expired is handled globally by coreRpcClient; any failure
            // here just means "no budget data", so stay silent rather than
            // guess. Never manufacture a warning from a failed read.
            if (err instanceof CoreRpcError && err.kind === 'auth_expired') {
              console.debug(`${LOG} fallback budget read skipped: session expired`);
              if (!cancelled) setFallbackUsage(null);
            } else {
              console.warn(`${LOG} fallback budget read failed kind=${errorKind(err)}`);
              if (!cancelled) setFallbackUsage(null);
            }
          }
        } else if (!cancelled) {
          // Not needed (BYO/local, or `useUsageState` already has the figure).
          console.debug(
            `${LOG} fallback budget read not needed ` +
              `(managed=${managed} hasUsage=${hasUsage} usageLoading=${usageLoading})`
          );
          setFallbackUsage(null);
        }
      } catch (err) {
        // Conservative on failure: an unknown provider is treated as
        // NOT managed, so a transient RPC error can never manufacture a
        // budget warning for a user who funds their own embeddings.
        console.warn(`${LOG} provider read failed kind=${errorKind(err)} — treating as unmanaged`);
        if (!cancelled) {
          setProvider(null);
          setFallbackUsage(null);
        }
      } finally {
        if (!cancelled) setProviderLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [reloadCount, isAuthenticated, hasUsage, usageLoading]);

  const isManagedEmbeddings = isManagedEmbeddingProvider(provider);

  // Without this the warning outlives its own fix. The banner is mounted for
  // the app's lifetime, so a single mount-time read means a user who follows
  // the CTA and switches to local Ollama keeps being told their memory is
  // broken until they restart the app — which would make the remediation look
  // like it did not work.
  //
  // Only re-polls while embeddings still bill against the managed budget, so
  // the majority of users (BYO key, local) cost nothing. The subscription
  // below covers the other direction.
  useEffect(() => {
    if (!isManagedEmbeddings) {
      console.debug(`${LOG} polling off (embeddings do not bill the managed budget)`);
      return;
    }
    console.debug(`${LOG} polling on every ${PROVIDER_RECHECK_MS}ms`);
    const id = window.setInterval(reload, PROVIDER_RECHECK_MS);
    return () => {
      console.debug(`${LOG} polling stopped`);
      window.clearInterval(id);
    };
  }, [isManagedEmbeddings, reload]);

  // Any usage refresh (sign-in, plan change, manual refresh) also re-reads the
  // provider, so a user who *switches onto* managed embeddings starts being
  // warned without waiting for a remount.
  useEffect(() => subscribeUsageRefresh(reload), [reload]);
  const isLoading = usageLoading || providerLoading;

  // Prefer `useUsageState`'s figure; fall back to the direct read for the
  // routed-away managed-embeddings case. When neither is available the session
  // never reached the billing API (signed out / offline) — claiming a budget
  // state from that is guesswork, so stay silent.
  const budget: DerivedBudget | null = teamUsage
    ? { pct: usagePct, exhausted: isBudgetExhausted }
    : fallbackUsage;

  const level =
    isLoading || !isManagedEmbeddings || !budget
      ? 'none'
      : embeddingBudgetLevel(budget.pct, budget.exhausted);

  return { level, pct: Math.round((budget?.pct ?? 0) * 100), isLoading, isManagedEmbeddings };
}
