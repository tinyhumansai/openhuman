/**
 * Onboarding step that gathers user context from connected integrations.
 *
 * Orchestrates the LinkedIn-enrichment pipeline directly in TypeScript:
 *
 *   1. Composio Gmail search (`tools_composio_execute` -> `GMAIL_FETCH_EMAILS`)
 *      to find a LinkedIn profile URL in the user's recent mail.
 *   2. Apify LinkedIn scrape — disabled (unreliable); stage is skipped.
 *   3. Persist a URL-only markdown via `learning_save_profile` with
 *      `summarize=true` so the core LLM compresses it into PROFILE.md.
 *
 * External calls still go through core (auth, proxy, billing). Only the
 * stage-by-stage orchestration lives in the renderer.
 */
import * as Sentry from '@sentry/react';
import { useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { callCoreRpc, getCoreRpcUrl, testCoreRpcConnection } from '../../../services/coreRpcClient';
import OnboardingNextButton from '../components/OnboardingNextButton';

/**
 * Threshold after which the building-profile UI swaps to the "still working"
 * staged copy (#2156). Real first-launch completions on slow M-series Macs
 * can take 30–40s, so users would otherwise see what looks like a stall at
 * the global RPC timeout boundary.
 */
const STILL_WORKING_THRESHOLD_MS = 30_000;

/** How often we probe `core.ping` after entering the still-working state. */
const ALIVE_PROBE_INTERVAL_MS = 5_000;

/**
 * Per-probe network budget. `testCoreRpcConnection` uses raw `fetch()` with
 * no built-in timeout, so we have to bound the probe ourselves — otherwise a
 * TCP black-hole (firewall drop, suspended laptop wake) lets the first probe
 * hang forever and the single-flight guard blocks every subsequent tick.
 */
const PROBE_TIMEOUT_MS = 3_000;

type AliveState = 'unknown' | 'probing' | 'alive' | 'unreachable';

interface ContextGatheringStepProps {
  connectedSources: string[];
  onNext: () => void | Promise<void>;
  onBack?: () => void;
}

/** Unwrap the RpcOutcome CLI envelope the core wraps around responses. */
function unwrapCliEnvelope<T>(value: unknown): T {
  if (
    value !== null &&
    typeof value === 'object' &&
    'result' in (value as Record<string, unknown>) &&
    'logs' in (value as Record<string, unknown>)
  ) {
    return (value as { result: T }).result;
  }
  return value as T;
}

interface Stage {
  id: 'gmail-search' | 'linkedin-scrape' | 'build-profile';
  labelKey: string;
}

const STAGES: Stage[] = [
  { id: 'gmail-search', labelKey: 'onboarding.contextGathering.stageGmail' },
  { id: 'linkedin-scrape', labelKey: 'onboarding.contextGathering.stageLinkedIn' },
  { id: 'build-profile', labelKey: 'onboarding.contextGathering.stageProfile' },
];

type StageStatus = 'pending' | 'active' | 'done' | 'skipped' | 'error';

// LinkedIn `comm/in/<slug>` (notification-email form) and `in/<slug>`
// (canonical) — same regex as `src/openhuman/learning/linkedin_enrichment.rs`.
const LINKEDIN_RE =
  /https?:\/\/(?:www\.|[a-z]{2,3}\.)?linkedin\.com\/(?:comm\/)?in\/([a-zA-Z0-9_-]+)/;

function canonicalLinkedInUrl(slug: string): string {
  return `https://www.linkedin.com/in/${slug}`;
}

function stageNow(): number {
  return globalThis.performance?.now() ?? Date.now();
}

function durationMs(startedAt: number): number {
  return Math.round(stageNow() - startedAt);
}

function errorReason(error: unknown): string {
  if (error instanceof Error && error.message.trim()) return error.message;
  if (typeof error === 'string' && error.trim()) return error;
  return 'unknown';
}

/** URL-safe base64 → utf-8 string (Gmail body parts arrive in this form). */
function decodeBase64Url(s: string): string {
  try {
    const padded = s.replace(/-/g, '+').replace(/_/g, '/');
    const pad = padded.length % 4 === 0 ? '' : '='.repeat(4 - (padded.length % 4));
    const bin = atob(padded + pad);
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    return new TextDecoder('utf-8').decode(bytes);
  } catch {
    return '';
  }
}

/**
 * Walk a Gmail-API-shaped message payload, decoding any base64 body parts,
 * and concatenate everything into a single searchable string.
 */
function extractSearchableText(message: unknown): string {
  const parts: string[] = [];
  const visit = (node: unknown) => {
    if (!node || typeof node !== 'object') return;
    const obj = node as Record<string, unknown>;
    if (typeof obj.messageText === 'string') parts.push(obj.messageText);
    if (typeof obj.snippet === 'string') parts.push(obj.snippet);
    const body = obj.body as Record<string, unknown> | undefined;
    if (body && typeof body.data === 'string') parts.push(decodeBase64Url(body.data));
    const subParts = obj.parts;
    if (Array.isArray(subParts)) for (const p of subParts) visit(p);
    const payload = obj.payload;
    if (payload) visit(payload);
  };
  visit(message);
  return parts.join('\n');
}

interface ComposioExecuteResult {
  successful: boolean;
  data: unknown;
  error?: string | null;
}

async function findLinkedInUrlViaComposio(): Promise<string | null> {
  console.debug('[onboarding:context] composio GMAIL_FETCH_EMAILS');
  const raw = await callCoreRpc<unknown>({
    method: 'openhuman.tools_composio_execute',
    params: {
      action: 'GMAIL_FETCH_EMAILS',
      params: { query: 'from:linkedin.com', max_results: 10 },
    },
  });
  const result = unwrapCliEnvelope<ComposioExecuteResult>(raw);
  if (!result.successful) {
    throw new Error(result.error ?? 'GMAIL_FETCH_EMAILS failed');
  }
  const data = result.data as { messages?: unknown[] } | null;
  const messages = Array.isArray(data?.messages) ? data!.messages : [];
  for (const msg of messages) {
    const text = extractSearchableText(msg);
    const m = text.match(LINKEDIN_RE);
    if (m) return canonicalLinkedInUrl(m[1]);
  }
  return null;
}

/**
 * First-launch profile compression on slow hardware (#2156) can run past the
 * global 30s RPC timeout while the core LLM compressor finishes. Use a
 * longer-but-still-bounded budget so users with a slow-but-alive core are
 * not parked on the post-login fallback when the call would otherwise
 * succeed. Real failures still abort within `SAVE_PROFILE_TIMEOUT_MS`.
 */
const SAVE_PROFILE_TIMEOUT_MS = 90_000;

async function saveProfile(markdown: string): Promise<void> {
  await callCoreRpc<unknown>({
    method: 'openhuman.learning_save_profile',
    params: { markdown, summarize: true },
    timeoutMs: SAVE_PROFILE_TIMEOUT_MS,
  });
}

const ContextGatheringStep = ({
  connectedSources,
  onNext,
  onBack: _onBack,
}: ContextGatheringStepProps) => {
  const { t } = useT();
  // Stage statuses are tracked in a ref — they drive pipeline branching only,
  // not rendering, so there is no need to trigger re-renders on each update.
  const stageStatusesRef = useRef<Record<string, StageStatus>>(
    Object.fromEntries(STAGES.map(s => [s.id, 'pending' as StageStatus]))
  );
  const [finished, setFinished] = useState(false);
  const [hasError, setHasError] = useState(false);
  // Staged "still working" mode kicks in after STILL_WORKING_THRESHOLD_MS so
  // a slow-but-alive first launch no longer looks like a stall (#2156).
  const [stillWorking, setStillWorking] = useState(false);
  const [aliveState, setAliveState] = useState<AliveState>('unknown');
  const backgroundClickedRef = useRef(false);
  const ranRef = useRef(false);

  const hasGmail = connectedSources.some(s => s.includes('gmail'));

  const setStage = (id: Stage['id'], status: StageStatus, duration?: number, reason?: string) => {
    stageStatusesRef.current = { ...stageStatusesRef.current, [id]: status };
    console.debug('[onboarding:context] stage status', {
      stage: id,
      status,
      durationMs: duration,
      reason,
    });
  };

  async function runPipeline() {
    const pipelineStartedAt = stageNow();
    console.debug('[onboarding:context] pipeline started');

    // Stage 1 — Gmail
    const gmailStartedAt = stageNow();
    setStage('gmail-search', 'active');
    let profileUrl: string | null;
    try {
      profileUrl = await findLinkedInUrlViaComposio();
      if (profileUrl) {
        setStage('gmail-search', 'done', durationMs(gmailStartedAt));
      } else {
        setStage('gmail-search', 'skipped', durationMs(gmailStartedAt), 'no_linkedin_url');
        setStage('linkedin-scrape', 'skipped');
        setStage('build-profile', 'skipped');
        console.debug('[onboarding:context] pipeline finished', {
          durationMs: durationMs(pipelineStartedAt),
          result: 'no_profile_url',
        });
        setFinished(true);
        return;
      }
    } catch (e) {
      const reason = errorReason(e);
      console.warn('[onboarding:context] gmail stage failed', {
        durationMs: durationMs(gmailStartedAt),
        reason,
      });
      setStage('gmail-search', 'error', durationMs(gmailStartedAt), reason);
      setStage('linkedin-scrape', 'skipped');
      setStage('build-profile', 'skipped');
      console.debug('[onboarding:context] pipeline finished', {
        durationMs: durationMs(pipelineStartedAt),
        result: 'gmail_error',
        reason,
      });
      setHasError(true);
      setFinished(true);
      return;
    }

    // Stage 2 — Apify LinkedIn scrape (disabled: unreliable, skipped during
    // profile build). PROFILE.md is built URL-only from stage 3.
    setStage('linkedin-scrape', 'skipped', 0, 'apify_disabled');
    const scrapedMarkdown = '';

    // Stage 3 — summarize + persist via core LLM compressor
    const profileStartedAt = stageNow();
    setStage('build-profile', 'active');
    try {
      const body = scrapedMarkdown.trim()
        ? scrapedMarkdown
        : `# User Profile\n\nLinkedIn: ${profileUrl}\n\n_Scrape returned no data._`;
      await saveProfile(body);
      setStage('build-profile', 'done', durationMs(profileStartedAt));
    } catch (e) {
      const reason = errorReason(e);
      console.warn('[onboarding:context] save_profile failed', {
        durationMs: durationMs(profileStartedAt),
        reason,
      });
      setStage('build-profile', 'error', durationMs(profileStartedAt), reason);
      setHasError(true);
    }

    console.debug('[onboarding:context] pipeline finished', {
      durationMs: durationMs(pipelineStartedAt),
      result: stageStatusesRef.current['build-profile'] === 'error' ? 'profile_error' : 'completed',
    });
    setFinished(true);
  }

  const continueToChat = () => {
    backgroundClickedRef.current = true;
    console.debug('[onboarding:context] user continued before pipeline completion', {
      finished,
      hasError,
      stages: stageStatusesRef.current,
    });
    void onNext();
  };

  // Auto-start pipeline on mount
  useEffect(() => {
    if (ranRef.current) return;
    ranRef.current = true;

    if (!hasGmail) {
      for (const s of STAGES) setStage(s.id, 'skipped');
      setFinished(true);
      return;
    }

    void runPipeline();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Staged "still working" trigger (#2156). After STILL_WORKING_THRESHOLD_MS
  // of pipeline runtime, swap to the calmer copy and start probing
  // `core.ping` so we can tell users whether the core is slow-but-alive vs
  // truly unreachable. Cleared on finish/error so completed runs never flip
  // to the slow-path UI retroactively.
  useEffect(() => {
    if (finished || !hasGmail || hasError) return;
    const timer = window.setTimeout(() => {
      setStillWorking(true);
    }, STILL_WORKING_THRESHOLD_MS);
    return () => window.clearTimeout(timer);
  }, [finished, hasGmail, hasError]);

  // Periodic alive probe while in still-working state. `core.ping` resolves
  // quickly even when the busy snapshot RPC is holding the worker, so a
  // green ping during a slow snapshot is exactly the alive-but-slow signal
  // users need to see. On cold start the bearer token may not be resolved
  // yet (IPC race) and the probe will come back 401 — that means auth is
  // still warming up, not that the core is down, so we treat 401 as `alive`.
  useEffect(() => {
    if (!stillWorking || finished || hasError) return;
    let cancelled = false;
    // Single-flight guard so a slow probe never gets shadowed by the next
    // 5s tick. Each probe also bounds itself with its own AbortController
    // (PROBE_TIMEOUT_MS) so a TCP-black-hole core (firewall, suspended
    // laptop) cannot leave us with a permanently in-flight fetch and a
    // permanently blocked guard — that would re-create exactly the
    // "Checking core connection…" forever failure mode this UI exists to
    // fix.
    let inFlight = false;

    const probe = async () => {
      if (cancelled || inFlight) return;
      inFlight = true;
      setAliveState(prev => (prev === 'unknown' ? 'probing' : prev));
      const controller = new AbortController();
      const timer = window.setTimeout(() => controller.abort(), PROBE_TIMEOUT_MS);
      try {
        const url = await getCoreRpcUrl();
        const response = await testCoreRpcConnection(url, undefined, { signal: controller.signal });
        if (!cancelled) {
          // 401 = auth not ready yet (cold-start IPC race), not a dead core.
          setAliveState(response.ok || response.status === 401 ? 'alive' : 'unreachable');
        }
      } catch {
        if (!cancelled) setAliveState('unreachable');
      } finally {
        window.clearTimeout(timer);
        inFlight = false;
      }
    };

    void probe();
    const interval = window.setInterval(probe, ALIVE_PROBE_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, [stillWorking, finished, hasError]);

  // Auto-navigate on successful completion (skip if user already clicked background link)
  useEffect(() => {
    if (finished && !hasError && !backgroundClickedRef.current) {
      const t = setTimeout(() => {
        void Promise.resolve(onNext()).catch(e => {
          console.warn('[onboarding:context] auto-advance failed', e);
          // Mirrors the manual click capture in ContextPage so the auto-
          // advance failure mode is not a Sentry blind spot (#2081). The
          // step tag distinguishes it from `continue-to-chat` clicks in
          // the dashboard.
          Sentry.captureException(e, {
            tags: { flow: 'onboarding-complete', step: 'auto-advance' },
          });
          setHasError(true);
        });
      }, 800);
      return () => clearTimeout(t);
    }
  }, [finished, hasError, onNext]);

  if (finished && hasError) {
    return (
      <div className="rounded-2xl border border-line bg-surface p-8 shadow-soft animate-fade-up">
        <div className="flex flex-col items-center justify-center gap-5">
          <h1 className="text-xl font-bold text-content">
            {t('onboarding.contextGathering.title')}
          </h1>
          <div className="w-full max-w-sm rounded-xl border border-primary-100 bg-primary-50/80 p-4 dark:border-primary-900/50 dark:bg-primary-950/30">
            <p className="text-sm text-content-secondary text-center leading-relaxed mb-4">
              {t('onboarding.contextGathering.errorDesc')}
            </p>
            <OnboardingNextButton
              label={t('onboarding.contextGathering.continueToChat')}
              onClick={continueToChat}
            />
          </div>
        </div>
      </div>
    );
  }

  // The slow-path UI must vanish as soon as the pipeline resolves, even
  // during the 800ms auto-advance window — otherwise a slow-success user
  // sees "still working…" copy after the work has actually finished.
  const showStillWorking = stillWorking && !finished && !hasError;
  const titleKey = showStillWorking
    ? 'onboarding.contextGathering.stillWorkingTitle'
    : 'onboarding.contextGathering.buildingProfile';
  const descKey = showStillWorking
    ? 'onboarding.contextGathering.stillWorkingDesc'
    : 'onboarding.contextGathering.buildingDesc';

  const aliveLabelKey: Record<AliveState, string> = {
    unknown: 'onboarding.contextGathering.coreAliveProbing',
    probing: 'onboarding.contextGathering.coreAliveProbing',
    alive: 'onboarding.contextGathering.coreAlive',
    unreachable: 'onboarding.contextGathering.coreUnreachable',
  };

  return (
    <div className="rounded-2xl border border-line bg-surface p-8 shadow-soft animate-fade-up">
      <div className="flex flex-col items-center justify-center gap-6 py-8">
        {/* Pulsing avatar silhouette */}
        <div className="w-20 h-20 rounded-full bg-gradient-to-r from-stone-300 via-stone-100 to-stone-300 bg-[length:200%_100%] animate-shimmer" />

        {/* Title */}
        <h1
          className="text-xl font-bold text-content animate-pulse"
          data-testid="context-gathering-title">
          {t(titleKey)}
        </h1>
        <p
          className="text-sm text-content-muted leading-relaxed text-center max-w-sm"
          data-testid="context-gathering-desc">
          {t(descKey)}
        </p>

        {/* Skeleton bars */}
        <div className="w-64 flex flex-col gap-3 mt-2">
          <div className="h-3 rounded-full bg-gradient-to-r from-stone-300 via-stone-100 to-stone-300 bg-[length:200%_100%] animate-shimmer" />
          <div className="h-3 w-3/4 rounded-full bg-gradient-to-r from-stone-300 via-stone-100 to-stone-300 bg-[length:200%_100%] animate-shimmer [animation-delay:150ms]" />
          <div className="h-3 w-1/2 rounded-full bg-gradient-to-r from-stone-300 via-stone-100 to-stone-300 bg-[length:200%_100%] animate-shimmer [animation-delay:300ms]" />
        </div>

        {/* Alive indicator — only while still-working state is active AND
            the pipeline hasn't finished/errored. Avoids flashing the probe
            during the 800ms auto-advance window after a slow success. */}
        {showStillWorking && (
          <div
            className="flex items-center gap-2 text-xs text-content-muted"
            data-testid="core-alive-indicator"
            data-alive-state={aliveState}>
            <span
              className={`inline-block h-2 w-2 rounded-full ${
                aliveState === 'alive'
                  ? 'bg-emerald-500'
                  : aliveState === 'unreachable'
                    ? 'bg-red-500'
                    : 'bg-surface-strong animate-pulse'
              }`}
              aria-hidden="true"
            />
            <span>{t(aliveLabelKey[aliveState])}</span>
          </div>
        )}

        {hasGmail && !finished && (
          <OnboardingNextButton
            label={t('onboarding.contextGathering.continueToChat')}
            onClick={continueToChat}
          />
        )}
      </div>
    </div>
  );
};

export default ContextGatheringStep;
