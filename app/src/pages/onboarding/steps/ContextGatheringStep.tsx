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
import { useRef, useState } from 'react';

import Button from '../../../components/ui/Button';
import WhatLeavesLink from '../../../features/privacy/WhatLeavesLink';
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
  { id: 'gmail-search', label: 'Reading your Gmail' },
  { id: 'linkedin-scrape', label: 'Researching you online' },
  { id: 'build-profile', label: 'Building your profile' },
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
  const [stageStatuses, setStageStatuses] = useState<Record<string, StageStatus>>(() => {
    const initial: Record<string, StageStatus> = {};
    for (const s of STAGES) initial[s.id] = 'pending';
    return initial;
  });
  const [stageDetails, setStageDetails] = useState<Record<string, string>>({});
  const [finished, setFinished] = useState(false);
  const [started, setStarted] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const ranRef = useRef(false);

  const hasGmail = connectedSources.some(s => s.includes('gmail'));

  const setStage = (id: Stage['id'], status: StageStatus, detail?: string) => {
    setStageStatuses(prev => ({ ...prev, [id]: status }));
    if (detail !== undefined) setStageDetails(prev => ({ ...prev, [id]: detail }));
  };

  const handleStart = () => {
    if (ranRef.current) return;
    ranRef.current = true;
    setStarted(true);

    if (!hasGmail) {
      const skipped: Record<string, StageStatus> = {};
      for (const s of STAGES) skipped[s.id] = 'skipped';
      setStageStatuses(skipped);
      setStageDetails({ 'gmail-search': 'Gmail not connected' });
      setFinished(true);
      return;
    }

    void runPipeline();
  };

  async function runPipeline() {
    console.debug('[onboarding:context] runPipeline started');

    // Stage 1 — Gmail
    setStage('gmail-search', 'active');
    let profileUrl: string | null;
    try {
      profileUrl = await findLinkedInUrlViaComposio();
      if (profileUrl) {
        setStage('gmail-search', 'done', profileUrl);
      } else {
        setStage('gmail-search', 'skipped', 'No LinkedIn URL found in mailbox');
        setStage('linkedin-scrape', 'skipped');
        setStage('build-profile', 'skipped');
        setFinished(true);
        return;
      }
    } catch (e) {
      console.warn('[onboarding:context] gmail stage failed', e);
      setStage('gmail-search', 'error', e instanceof Error ? e.message : String(e));
      setStage('linkedin-scrape', 'skipped');
      setStage('build-profile', 'skipped');
      setFinished(true);
      return;
    }

    // Stage 2 — Apify LinkedIn scrape
    setStage('linkedin-scrape', 'active');
    let scrapedMarkdown = '';
    try {
      scrapedMarkdown = await apifyScrapeLinkedIn(profileUrl);
      setStage(
        'linkedin-scrape',
        scrapedMarkdown.trim() ? 'done' : 'skipped',
        scrapedMarkdown.trim() ? 'Profile scraped' : 'No scraped data'
      );
    } catch (e) {
      console.warn('[onboarding:context] apify_linkedin_scrape stage failed', e);
      setStage('linkedin-scrape', 'error', e instanceof Error ? e.message : String(e));
      // Continue — save_profile can still write a URL-only file.
    }

    // Stage 3 — summarize + persist via core LLM compressor
    setStage('build-profile', 'active');
    try {
      const body = scrapedMarkdown.trim()
        ? scrapedMarkdown
        : `# User Profile\n\nLinkedIn: ${profileUrl}\n\n_Scrape returned no data._`;
      await saveProfile(body);
      setStage('build-profile', 'done', 'PROFILE.md saved');
    } catch (e) {
      console.warn('[onboarding:context] save_profile failed', e);
      setStage('build-profile', 'error', e instanceof Error ? e.message : String(e));
    }

    setFinished(true);
  }

  const completedCount = STAGES.filter(s => {
    const st = stageStatuses[s.id];
    return st === 'done' || st === 'skipped' || st === 'error';
  }).length;
  const progressPercent = Math.round((completedCount / STAGES.length) * 100);
  const isRunning = !finished;
  const activeStageIdx = STAGES.findIndex(s => stageStatuses[s.id] === 'active');

  const handleContinue = async () => {
    setError(null);
    try {
      await onNext();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Something went wrong.');
    }
  };

  if (!started) {
    return (
      <div
        className="rounded-2xl border border-stone-200 bg-white p-8 shadow-soft animate-fade-up"
        data-testid="context-gathering-intro">
        <div className="text-center mb-5">
          <h1 className="text-xl font-bold mb-2 text-stone-900">Getting To Know You</h1>
          <p className="text-stone-500 text-sm leading-relaxed max-w-sm mx-auto">
            I'm going to build a short profile about you so the first conversation isn't cold.
          </p>
        </div>
        <div className="rounded-xl border border-stone-100 bg-stone-50 p-4 mb-5 text-sm text-stone-600 leading-relaxed">
          {hasGmail ? (
            <>
              Using your <span className="font-medium text-stone-900">connected Gmail</span> we will
              build a short profile about you. Everything happens in your device itself for maximum
              privacy.
            </>
          ) : (
            <>You haven't connected Gmail. Nothing to read. You can skip this step.</>
          )}
        </div>
        <OnboardingNextButton label={hasGmail ? "Let's go!" : 'Continue'} onClick={handleStart} />
        <div className="mt-3 flex items-center justify-between gap-3">
          <Button variant="ghost" size="sm" onClick={() => void onNext()}>
            Skip for now
          </Button>
          <WhatLeavesLink />
        </div>
      </div>
    );
  }

  return (
    <div className="rounded-2xl border border-stone-200 bg-white p-8 shadow-soft animate-fade-up">
      <div className="text-center mb-5">
        <h1 className="text-xl font-bold mb-2 text-stone-900">
          {finished ? 'Context Ready' : 'Reading your connected accounts'}
        </h1>
        <p className="text-stone-500 text-sm">
          {finished
            ? 'Short profile saved locally. Ready to chat.'
            : 'Working from what you already connected…'}
        </p>
      </div>

      <div className="mb-5">
        <div className="h-2 w-full overflow-hidden rounded-full bg-stone-100">
          {isRunning ? (
            <div className="h-full w-full rounded-full bg-primary-400/60 animate-pulse" />
          ) : (
            <div
              className="h-full rounded-full bg-primary-500 transition-all duration-500 ease-out"
              style={{ width: `${finished ? 100 : Math.max(progressPercent, 8)}%` }}
            />
          )}
        </div>
        {isRunning && activeStageIdx >= 0 && (
          <p className="mt-2 text-xs text-primary-600 text-center animate-pulse">
            {STAGES[activeStageIdx].label}...
          </p>
        )}
      </div>

      <div className="mb-5 space-y-2">
        {STAGES.map((stage, idx) => {
          const status = stageStatuses[stage.id];
          const detail = stageDetails[stage.id];
          const displayStatus =
            isRunning && status === 'pending' && idx <= (activeStageIdx < 0 ? 0 : activeStageIdx)
              ? 'active'
              : status;

          return (
            <div
              key={stage.id}
              className="flex items-start gap-3 rounded-xl border border-stone-100 px-3 py-2.5">
              <div className="mt-0.5 flex-shrink-0">
                {displayStatus === 'done' && (
                  <div className="h-4 w-4 rounded-full bg-sage-500 flex items-center justify-center">
                    <svg
                      className="h-2.5 w-2.5 text-white"
                      fill="none"
                      stroke="currentColor"
                      viewBox="0 0 24 24">
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        strokeWidth={3}
                        d="M5 13l4 4L19 7"
                      />
                    </svg>
                  </div>
                )}
                {displayStatus === 'active' && (
                  <div className="h-4 w-4 rounded-full border-2 border-primary-500 border-t-transparent animate-spin" />
                )}
                {displayStatus === 'pending' && (
                  <div className="h-4 w-4 rounded-full border-2 border-stone-200" />
                )}
                {displayStatus === 'skipped' && (
                  <div className="h-4 w-4 rounded-full bg-stone-200 flex items-center justify-center">
                    <span className="text-[8px] text-stone-400">--</span>
                  </div>
                )}
                {displayStatus === 'error' && (
                  <div className="h-4 w-4 rounded-full bg-amber-400 flex items-center justify-center">
                    <span className="text-[8px] text-white font-bold">!</span>
                  </div>
                )}
              </div>

              <div className="min-w-0 flex-1">
                <p
                  className={`text-sm font-medium ${
                    displayStatus === 'active'
                      ? 'text-stone-900'
                      : displayStatus === 'done'
                        ? 'text-sage-700'
                        : displayStatus === 'error'
                          ? 'text-amber-700'
                          : 'text-stone-400'
                  }`}>
                  {stage.label}
                </p>
                {detail && !isRunning && (
                  <p
                    className={`mt-0.5 text-xs truncate ${
                      displayStatus === 'error' ? 'text-amber-500' : 'text-stone-400'
                    }`}>
                    {detail}
                  </p>
                )}
              </div>
            </div>
          );
        })}
      </div>

      {error && <p className="text-coral-400 text-sm mb-3 text-center">{error}</p>}

      <OnboardingNextButton onClick={handleContinue} disabled={!finished} label="Continue" />
      <div className="mt-3 flex justify-center">
        <WhatLeavesLink />
      </div>
    </div>
  );
};

export default ContextGatheringStep;
