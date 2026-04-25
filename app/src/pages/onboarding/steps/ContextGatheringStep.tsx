/**
 * Onboarding step that gathers user context from connected integrations.
 *
 * Calls the Rust-side `learning.linkedin_enrichment` controller which
 * runs the full pipeline: Gmail search -> LinkedIn extraction -> Apify
 * scrape -> LLM summarisation -> PROFILE.md. The frontend shows a
 * progress animation while the pipeline runs and displays the log when
 * it finishes.
 */
import { invoke } from '@tauri-apps/api/core';
import { useRef, useState } from 'react';

import WebviewHost from '../../../components/accounts/WebviewHost';
import Button from '../../../components/ui/Button';
import WhatLeavesLink from '../../../features/privacy/WhatLeavesLink';
import { callCoreRpc } from '../../../services/coreRpcClient';
import OnboardingNextButton from '../components/OnboardingNextButton';

interface ContextGatheringStepProps {
  connectedSources: string[];
  /**
   * Account id of the gmail webview the user signed into in SkillsStep.
   * Required for the webview-driven Gmail search path; absent means the
   * pipeline can't run stage 1 and will skip enrichment.
   */
  gmailAccountId?: string;
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

interface EnrichmentResult {
  profile_url: string | null;
  profile_data: unknown | null;
  log: string[];
}

// ── Visual stage definitions (driven by pipeline log lines) ──────────

interface Stage {
  id: string;
  label: string;
  /** Substring to look for in log lines to mark this stage done. */
  doneSignal: string;
  /** If this appears in a log line, the stage is an error. */
  errorSignal?: string;
  /** If this appears, mark as skipped. */
  skipSignal?: string;
}

const STAGES: Stage[] = [
  {
    id: 'gmail-search',
    label: 'Indexing your GMail',
    doneSignal: 'Found LinkedIn profile',
    errorSignal: 'Gmail search failed',
    skipSignal: 'No LinkedIn profile URL',
  },
  {
    id: 'apify-scrape',
    label: 'Finding your LinkedIn',
    doneSignal: 'profile scraped successfully',
    errorSignal: 'scrape failed',
  },
  {
    id: 'build-profile',
    label: 'Building your profile',
    doneSignal: 'PROFILE.md written',
    errorSignal: 'Failed to write PROFILE',
  },
];

type StageStatus = 'pending' | 'active' | 'done' | 'skipped' | 'error';

// ── Component ────────────────────────────────────────────────────────

const ContextGatheringStep = ({
  connectedSources,
  gmailAccountId,
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

  // Either path counts as "has gmail": a webview-driven session we can
  // drive via CDP, or a Composio source the core can read via API.
  const hasGmail = !!gmailAccountId || connectedSources.some(s => s.includes('gmail'));

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
    console.debug('[onboarding:context] runPipeline started', { gmailAccountId });
    setStageStatuses(prev => ({ ...prev, 'gmail-search': 'active' }));

    // Stage 1 (webview path): drive the live Gmail UI to search for
    // `from:linkedin.com` and regex-extract the user's profile URL out
    // of the matched bodies. Pure CDP — no JS injection, no Composio.
    let profileUrl: string | null = null;
    if (gmailAccountId) {
      try {
        profileUrl =
          (await invoke<string | null>('gmail_find_linkedin_profile_url', {
            accountId: gmailAccountId,
          })) ?? null;
        console.debug('[onboarding:context] webview gmail search result', { profileUrl });
        setStageStatuses(prev => ({ ...prev, 'gmail-search': profileUrl ? 'done' : 'skipped' }));
        setStageDetails(prev => ({
          ...prev,
          'gmail-search': profileUrl ?? 'No LinkedIn email found in mailbox',
        }));
      } catch (e) {
        console.warn('[onboarding:context] webview gmail search failed', e);
        const errMsg = e instanceof Error ? e.message : String(e);
        setStageStatuses(prev => ({ ...prev, 'gmail-search': 'error' }));
        setStageDetails(prev => ({ ...prev, 'gmail-search': errMsg }));
      }
    } else {
      console.debug('[onboarding:context] no gmailAccountId — skipping stage 1');
      setStageStatuses(prev => ({ ...prev, 'gmail-search': 'skipped' }));
      setStageDetails(prev => ({ ...prev, 'gmail-search': 'Gmail not connected' }));
    }

    // Stages 2 + 3 (core pipeline): hand the URL we found to the core
    // RPC, which scrapes via Apify and writes PROFILE.md. The core
    // method now accepts an optional preset profile_url so we can skip
    // its built-in Composio-driven search.
    if (!profileUrl) {
      // No URL → mark downstream stages skipped and finish.
      setStageStatuses(prev => ({
        ...prev,
        'apify-scrape': 'skipped',
        'build-profile': 'skipped',
      }));
      setFinished(true);
      return;
    }

    setStageStatuses(prev => ({ ...prev, 'apify-scrape': 'active' }));
    try {
      console.debug('[onboarding:context] calling learning_linkedin_enrichment with preset URL');
      const raw = await callCoreRpc<unknown>({
        method: 'openhuman.learning_linkedin_enrichment',
        params: { profile_url: profileUrl },
      });
      const result = unwrapCliEnvelope<EnrichmentResult>(raw);
      console.debug('[onboarding:context] enrichment result', {
        profileUrl: result.profile_url,
        logLines: result.log.length,
        hasProfileData: result.profile_data !== null,
      });
      // Apply just the apify-scrape + build-profile stages; we already
      // settled gmail-search above.
      applyLogToStages(result.log, result.profile_url, ['apify-scrape', 'build-profile']);
    } catch (e) {
      console.debug('[onboarding:context] enrichment error', e);
      const errMsg = e instanceof Error ? e.message : 'Pipeline failed';
      setStageStatuses(prev => {
        const next = { ...prev };
        for (const s of STAGES) {
          if (s.id === 'gmail-search') continue;
          if (next[s.id] === 'pending' || next[s.id] === 'active') {
            next[s.id] = 'error';
          }
        }
        return next;
      });
      setStageDetails(prev => ({ ...prev, 'apify-scrape': errMsg }));
    }

    setFinished(true);
  }

  function applyLogToStages(
    log: string[],
    profileUrl: string | null,
    onlyStages?: readonly string[]
  ) {
    const allowedStages = onlyStages ? new Set(onlyStages) : null;
    const nextStatusPatch: Record<string, StageStatus> = {};
    const nextDetailPatch: Record<string, string> = {};

    for (const stage of STAGES) {
      if (allowedStages && !allowedStages.has(stage.id)) continue;

      let status: StageStatus = 'skipped';
      let detail = '';
      for (const line of log) {
        if (stage.skipSignal && line.includes(stage.skipSignal)) {
          status = 'skipped';
          detail = line;
          break;
        }
        if (stage.errorSignal && line.includes(stage.errorSignal)) {
          status = 'error';
          detail = line;
          break;
        }
        if (line.includes(stage.doneSignal)) {
          status = 'done';
          detail = line;
          break;
        }
      }
      nextStatusPatch[stage.id] = status;
      if (detail) nextDetailPatch[stage.id] = detail;
    }

    // If we found a profile URL and the gmail-search stage is being
    // updated this pass, surface the URL as its detail.
    if (profileUrl && nextStatusPatch['gmail-search'] && !nextDetailPatch['gmail-search']) {
      nextDetailPatch['gmail-search'] = profileUrl;
    }

    setStageStatuses(prev => ({ ...prev, ...nextStatusPatch }));
    setStageDetails(prev => ({ ...prev, ...nextDetailPatch }));
  }

  // ── Derived progress ──────────────────────────────────────────────

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

      {/* Progress bar */}
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

      {/* Stage list */}
      <div className="mb-5 space-y-2">
        {STAGES.map((stage, idx) => {
          const status = stageStatuses[stage.id];
          const detail = stageDetails[stage.id];
          // While pipeline is running, show stages up to current as active.
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

      {/*
        Offscreen-but-not-hidden gmail webview. The kept-alive webview
        from SkillsStep was hidden by the modal's WebviewHost cleanup —
        but a hidden CEF surface stops rendering, so CDP can't drive
        Gmail's UI for the search. Mounting another WebviewHost here
        triggers the warm-reuse path in `openWebviewAccount` which
        repositions the same webview to *these* bounds. Setting them
        to (-10000, -10000) at full size keeps Chromium's occlusion
        detection happy (NSView is in the window's view hierarchy and
        not hidden) so the page keeps rendering, but the user never
        sees it. Mounted only while the pipeline is running and
        unmounted on `finished` so the webview hides itself again
        before completeAndExit purges it.
      */}
      {gmailAccountId && isRunning && (
        <div
          aria-hidden="true"
          style={{
            position: 'fixed',
            left: '-10000px',
            top: '-10000px',
            width: '1280px',
            height: '900px',
            pointerEvents: 'none',
          }}>
          <WebviewHost accountId={gmailAccountId} provider="gmail" />
        </div>
      )}
    </div>
  );
};

export default ContextGatheringStep;
