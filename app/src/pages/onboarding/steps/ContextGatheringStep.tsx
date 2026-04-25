/**
 * Onboarding step that gathers user context from connected integrations.
 *
 * Calls the Rust-side `learning.linkedin_enrichment` controller which
 * runs the full pipeline: Composio Gmail search -> LinkedIn extraction
 * -> Apify scrape -> LLM summarisation -> PROFILE.md. The frontend shows
 * a progress animation while the pipeline runs and displays the log when
 * it finishes.
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

interface EnrichmentResult {
  profile_url: string | null;
  profile_data: unknown | null;
  log: string[];
}

interface Stage {
  id: string;
  label: string;
  doneSignal: string;
  errorSignal?: string;
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
    setStageStatuses(prev => ({ ...prev, 'gmail-search': 'active' }));

    try {
      const raw = await callCoreRpc<unknown>({
        method: 'openhuman.learning_linkedin_enrichment',
        params: {},
      });
      const result = unwrapCliEnvelope<EnrichmentResult>(raw);
      console.debug('[onboarding:context] enrichment result', {
        profileUrl: result.profile_url,
        logLines: result.log.length,
        hasProfileData: result.profile_data !== null,
      });
      applyLogToStages(result.log, result.profile_url);
    } catch (e) {
      console.debug('[onboarding:context] enrichment error', e);
      const errMsg = e instanceof Error ? e.message : 'Pipeline failed';
      setStageStatuses(prev => {
        const next = { ...prev };
        for (const s of STAGES) {
          if (next[s.id] === 'pending' || next[s.id] === 'active') {
            next[s.id] = 'error';
          }
        }
        return next;
      });
      setStageDetails(prev => ({ ...prev, 'gmail-search': errMsg }));
    }

    setFinished(true);
  }

  function applyLogToStages(log: string[], profileUrl: string | null) {
    const nextStatusPatch: Record<string, StageStatus> = {};
    const nextDetailPatch: Record<string, string> = {};

    for (const stage of STAGES) {
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

    if (profileUrl && !nextDetailPatch['gmail-search']) {
      nextDetailPatch['gmail-search'] = profileUrl;
    }

    setStageStatuses(prev => ({ ...prev, ...nextStatusPatch }));
    setStageDetails(prev => ({ ...prev, ...nextDetailPatch }));
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
