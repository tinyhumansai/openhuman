/**
 * Onboarding step that gathers user context from connected integrations.
 *
 * Orchestrates the LinkedIn-enrichment pipeline directly in TypeScript:
 *
 *   1. Composio Gmail search (`tools_composio_execute` -> `GMAIL_FETCH_EMAILS`)
 *      to find a LinkedIn profile URL in the user's recent mail.
 *   2. Apify LinkedIn scrape (`tools_apify_linkedin_scrape`) to pull a
 *      structured public profile snapshot and render it as markdown.
 *   3. Persist the assembled markdown via `learning_save_profile` with
 *      `summarize=true` so the core LLM compresses it into PROFILE.md.
 *
 * External calls still go through core (auth, proxy, billing). Only the
 * stage-by-stage orchestration lives in the renderer.
 */
import { useEffect, useRef, useState } from 'react';

import { callCoreRpc } from '../../../services/coreRpcClient';
import OnboardingNextButton from '../components/OnboardingNextButton';

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
  label: string;
}

const STAGES: Stage[] = [
  { id: 'gmail-search', label: 'Processing your Gmail' },
  { id: 'linkedin-scrape', label: 'Working on your LinkedIn' },
  { id: 'build-profile', label: 'Building your Profile' },
];

type StageStatus = 'pending' | 'active' | 'done' | 'skipped' | 'error';

// LinkedIn `comm/in/<slug>` (notification-email form) and `in/<slug>`
// (canonical) — same regex as `src/openhuman/learning/linkedin_enrichment.rs`.
const LINKEDIN_RE =
  /https?:\/\/(?:www\.|[a-z]{2,3}\.)?linkedin\.com\/(?:comm\/)?in\/([a-zA-Z0-9_-]+)/;

function canonicalLinkedInUrl(slug: string): string {
  return `https://www.linkedin.com/in/${slug}`;
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

async function apifyScrapeLinkedIn(profileUrl: string): Promise<string> {
  console.debug('[onboarding:context] apify_linkedin_scrape', { profileUrl });
  const raw = await callCoreRpc<unknown>({
    method: 'openhuman.tools_apify_linkedin_scrape',
    params: { profile_url: profileUrl },
  });
  const result = unwrapCliEnvelope<{ data: unknown; markdown: string }>(raw);
  return result.markdown ?? '';
}

async function saveProfile(markdown: string): Promise<void> {
  await callCoreRpc<unknown>({
    method: 'openhuman.learning_save_profile',
    params: { markdown, summarize: true },
  });
}

const ContextGatheringStep = ({
  connectedSources,
  onNext,
  onBack: _onBack,
}: ContextGatheringStepProps) => {
  // Stage statuses are tracked in a ref — they drive pipeline branching only,
  // not rendering, so there is no need to trigger re-renders on each update.
  const stageStatusesRef = useRef<Record<string, StageStatus>>(
    Object.fromEntries(STAGES.map(s => [s.id, 'pending' as StageStatus]))
  );
  const [finished, setFinished] = useState(false);
  const [hasError, setHasError] = useState(false);
  const [showBackgroundLink, setShowBackgroundLink] = useState(false);
  const backgroundClickedRef = useRef(false);
  const ranRef = useRef(false);

  const hasGmail = connectedSources.some(s => s.includes('gmail'));

  const setStage = (id: Stage['id'], status: StageStatus) => {
    stageStatusesRef.current = { ...stageStatusesRef.current, [id]: status };
  };

  async function runPipeline() {
    console.debug('[onboarding:context] runPipeline started');

    // Stage 1 — Gmail
    setStage('gmail-search', 'active');
    let profileUrl: string | null;
    try {
      profileUrl = await findLinkedInUrlViaComposio();
      if (profileUrl) {
        setStage('gmail-search', 'done');
      } else {
        setStage('gmail-search', 'skipped');
        setStage('linkedin-scrape', 'skipped');
        setStage('build-profile', 'skipped');
        setFinished(true);
        return;
      }
    } catch (e) {
      console.warn('[onboarding:context] gmail stage failed', e);
      setStage('gmail-search', 'error');
      setStage('linkedin-scrape', 'skipped');
      setStage('build-profile', 'skipped');
      setHasError(true);
      setFinished(true);
      return;
    }

    // Stage 2 — Apify LinkedIn scrape
    setStage('linkedin-scrape', 'active');
    let scrapedMarkdown = '';
    try {
      scrapedMarkdown = await apifyScrapeLinkedIn(profileUrl);
      setStage('linkedin-scrape', scrapedMarkdown.trim() ? 'done' : 'skipped');
    } catch (e) {
      console.warn('[onboarding:context] apify_linkedin_scrape stage failed', e);
      setStage('linkedin-scrape', 'error');
      // Continue — save_profile can still write a URL-only file.
    }

    // Stage 3 — summarize + persist via core LLM compressor
    setStage('build-profile', 'active');
    try {
      const body = scrapedMarkdown.trim()
        ? scrapedMarkdown
        : `# User Profile\n\nLinkedIn: ${profileUrl}\n\n_Scrape returned no data._`;
      await saveProfile(body);
      setStage('build-profile', 'done');
    } catch (e) {
      console.warn('[onboarding:context] save_profile failed', e);
      setStage('build-profile', 'error');
      setHasError(true);
    }

    setFinished(true);
  }

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

  // Auto-navigate on successful completion (skip if user already clicked background link)
  useEffect(() => {
    if (finished && !hasError && !backgroundClickedRef.current) {
      const t = setTimeout(() => {
        void Promise.resolve(onNext()).catch(e => {
          console.warn('[onboarding:context] auto-advance failed', e);
          setHasError(true);
        });
      }, 800);
      return () => clearTimeout(t);
    }
  }, [finished, hasError, onNext]);

  // Show "Keep building in background" link after 10s
  useEffect(() => {
    if (!hasGmail || finished) return;
    const t = setTimeout(() => setShowBackgroundLink(true), 10_000);
    return () => clearTimeout(t);
  }, [hasGmail, finished]);

  if (finished && hasError) {
    return (
      <div className="rounded-2xl border border-stone-200 bg-white p-8 shadow-soft animate-fade-up">
        <div className="flex flex-col items-center justify-center gap-5">
          <h1 className="text-xl font-bold text-stone-900">Almost there!</h1>
          <p className="text-sm text-stone-600 text-center max-w-xs leading-relaxed">
            We couldn&apos;t build your full profile right now, but that&apos;s okay — you can
            always update it later.
          </p>
          <OnboardingNextButton label="Continue" onClick={() => void onNext()} />
        </div>
      </div>
    );
  }

  return (
    <div className="rounded-2xl border border-stone-200 bg-white p-8 shadow-soft animate-fade-up">
      <div className="flex flex-col items-center justify-center gap-6 py-8">
        {/* Pulsing avatar silhouette */}
        <div className="w-20 h-20 rounded-full bg-gradient-to-r from-stone-300 via-stone-100 to-stone-300 bg-[length:200%_100%] animate-shimmer" />

        {/* Title */}
        <h1 className="text-xl font-bold text-stone-900 animate-pulse">Building your profile...</h1>
        <p className="text-sm text-stone-500 leading-relaxed">This will only take a moment.</p>

        {/* Skeleton bars */}
        <div className="w-64 flex flex-col gap-3 mt-2">
          <div className="h-3 rounded-full bg-gradient-to-r from-stone-300 via-stone-100 to-stone-300 bg-[length:200%_100%] animate-shimmer" />
          <div className="h-3 w-3/4 rounded-full bg-gradient-to-r from-stone-300 via-stone-100 to-stone-300 bg-[length:200%_100%] animate-shimmer [animation-delay:150ms]" />
          <div className="h-3 w-1/2 rounded-full bg-gradient-to-r from-stone-300 via-stone-100 to-stone-300 bg-[length:200%_100%] animate-shimmer [animation-delay:300ms]" />
        </div>

        {showBackgroundLink && (
          <button
            type="button"
            className="animate-fade-in text-sm text-ocean-600 hover:text-ocean-700 underline underline-offset-2 transition-colors mt-2"
            onClick={() => {
              backgroundClickedRef.current = true;
              void onNext();
            }}>
            Keep building in background &amp; continue
          </button>
        )}
      </div>
    </div>
  );
};

export default ContextGatheringStep;
