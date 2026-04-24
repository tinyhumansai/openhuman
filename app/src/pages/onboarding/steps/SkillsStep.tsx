import { useState } from 'react';

import ComposioConnectModal from '../../../components/composio/ComposioConnectModal';
import {
  composioToolkitMeta,
  type ComposioToolkitMeta,
  KNOWN_COMPOSIO_TOOLKITS,
} from '../../../components/composio/toolkitMeta';
import { useComposioIntegrations } from '../../../lib/composio/hooks';
import { canonicalizeComposioToolkitSlug } from '../../../lib/composio/toolkitSlug';
import { type ComposioConnection, deriveComposioState } from '../../../lib/composio/types';
import OnboardingNextButton from '../components/OnboardingNextButton';

interface SkillsStepProps {
  onNext: (connectedSources: string[]) => void | Promise<void>;
  onBack?: () => void;
}

// ── Status helpers (matches Skills page vocabulary) ──────────────────────

function statusDotClass(connection: ComposioConnection | undefined): string {
  switch (deriveComposioState(connection)) {
    case 'connected':
      return 'bg-sage-500';
    case 'pending':
      return 'bg-amber-500 animate-pulse';
    case 'error':
      return 'bg-coral-500';
    default:
      return 'bg-stone-300';
  }
}

function statusLabel(connection: ComposioConnection | undefined): string {
  switch (deriveComposioState(connection)) {
    case 'connected':
      return 'Connected';
    case 'pending':
      return 'Connecting';
    case 'error':
      return 'Error';
    default:
      return '';
  }
}

function statusColor(connection: ComposioConnection | undefined): string {
  switch (deriveComposioState(connection)) {
    case 'connected':
      return 'text-sage-600';
    case 'pending':
      return 'text-amber-600';
    case 'error':
      return 'text-coral-600';
    default:
      return 'text-stone-400';
  }
}

const SkillsStep = ({ onNext, onBack: _onBack }: SkillsStepProps) => {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [activeToolkit, setActiveToolkit] = useState<ComposioToolkitMeta | null>(null);

  const {
    toolkits: backendToolkits,
    connectionByToolkit,
    loading: composioLoading,
    error: composioError,
    refresh: refreshComposio,
  } = useComposioIntegrations();

  // Keep onboarding opinionated: show a small curated set of high-value
  // integrations, but never hide them just because the live Composio allowlist
  // hasn't loaded yet or temporarily returns an empty list.
  const ONBOARDING_SLUGS = ['gmail', 'googlecalendar', 'googledrive', 'notion'] as const;
  const normalizedBackendToolkits = backendToolkits.map(canonicalizeComposioToolkitSlug);
  const fallbackToolkits = ONBOARDING_SLUGS.filter(slug => KNOWN_COMPOSIO_TOOLKITS.includes(slug));
  const effectiveToolkits =
    normalizedBackendToolkits.length > 0
      ? ONBOARDING_SLUGS.filter(slug => normalizedBackendToolkits.includes(slug))
      : fallbackToolkits;
  const displayToolkits: ComposioToolkitMeta[] = effectiveToolkits.map(slug =>
    composioToolkitMeta(slug)
  );

  // Only count connections for the displayed toolkits.
  const connectedCount = displayToolkits.filter(t => {
    const conn = connectionByToolkit.get(t.slug);
    return conn && deriveComposioState(conn) === 'connected';
  }).length;

  const handleFinish = async () => {
    console.debug('[onboarding:skills] handleSkillsNext', { displayToolkits, connectedCount });
    setError(null);
    setLoading(true);
    try {
      // Only include connections for displayed toolkit slugs.
      const displaySlugs = new Set(displayToolkits.map(t => t.slug));
      const connectedSources = Array.from(connectionByToolkit.entries())
        .filter(([slug, c]) => displaySlugs.has(slug) && deriveComposioState(c) === 'connected')
        .map(([slug]) => `composio:${slug}`);
      await onNext(connectedSources);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Something went wrong. Please try again.');
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="rounded-2xl border border-stone-200 bg-white p-8 shadow-soft animate-fade-up">
      <div className="text-center mb-4">
        <h1 className="text-xl font-bold mb-2 text-stone-900">Connect your tools</h1>
        <p className="text-stone-600 text-sm">
          Connect the services you already use so OpenHuman can build context for your agent. Your
          data is saved locally and never leaves your device.
        </p>
      </div>

      {/* Integration cards */}
      <div className="mb-4 space-y-2">
        {composioError ? (
          <div className="rounded-xl border border-amber-200 bg-amber-50 p-4 text-center">
            <p className="text-sm text-amber-700 mb-2">Could not load connections</p>
            <button
              type="button"
              onClick={() => void refreshComposio()}
              className="text-xs font-medium text-amber-800 border border-amber-300 rounded-lg px-3 py-1 hover:bg-amber-100 transition-colors">
              Retry
            </button>
          </div>
        ) : composioLoading && displayToolkits.length === 0 ? (
          <div className="rounded-xl border border-stone-100 bg-stone-50 p-4 text-center">
            <p className="text-sm text-stone-400 animate-pulse">Loading connections…</p>
          </div>
        ) : (
          <>
            {displayToolkits.map(meta => {
              const connection = connectionByToolkit.get(meta.slug);
              const state = deriveComposioState(connection);
              const isConnected = state === 'connected';
              const isPending = state === 'pending';
              const label = statusLabel(connection);

              return (
                <button
                  key={meta.slug}
                  type="button"
                  onClick={() => setActiveToolkit(meta)}
                  className="w-full flex items-center gap-3 rounded-xl border border-stone-100 bg-white p-3 transition-colors hover:bg-stone-50 text-left">
                  {/* Icon */}
                  <div className="flex h-8 w-8 flex-shrink-0 items-center justify-center text-lg">
                    {meta.icon}
                  </div>

                  {/* Text */}
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="truncate text-sm font-semibold text-stone-900">
                        {meta.name}
                      </span>
                      {label && (
                        <>
                          <div
                            className={`h-1.5 w-1.5 flex-shrink-0 rounded-full ${statusDotClass(connection)}`}
                          />
                          <span className={`flex-shrink-0 text-xs ${statusColor(connection)}`}>
                            {label}
                          </span>
                        </>
                      )}
                    </div>
                    <p className="mt-0.5 line-clamp-1 text-xs leading-relaxed text-stone-500">
                      {meta.description}
                    </p>
                  </div>

                  {/* CTA badge */}
                  <span
                    className={`flex-shrink-0 rounded-lg border px-3 py-1.5 text-[11px] font-medium transition-colors ${
                      isConnected
                        ? 'border-sage-200 bg-sage-50 text-sage-700'
                        : isPending
                          ? 'border-amber-200 bg-amber-50 text-amber-700'
                          : 'border-primary-200 bg-primary-50 text-primary-700'
                    }`}>
                    {isConnected ? 'Manage' : isPending ? 'Waiting' : 'Connect'}
                  </span>
                </button>
              );
            })}

            {/* More connections hint */}
            <div className="rounded-xl border border-stone-100 bg-stone-50 px-3 py-2.5 text-center">
              <p className="text-xs text-stone-400">
                Notion, Slack, GitHub, and more available after setup
              </p>
            </div>
          </>
        )}
      </div>

      {connectedCount > 0 && (
        <p className="text-xs text-sage-600 text-center mb-3">
          {connectedCount} connection{connectedCount > 1 ? 's' : ''} active
        </p>
      )}

      {error && <p className="text-coral-400 text-sm mb-3 text-center">{error}</p>}

      <OnboardingNextButton
        onClick={handleFinish}
        loading={loading}
        loadingLabel="Loading..."
        label={connectedCount > 0 ? 'Continue' : 'Skip for Now'}
      />

      {/* Composio OAuth modal */}
      {activeToolkit && (
        <ComposioConnectModal
          toolkit={activeToolkit}
          connection={connectionByToolkit.get(activeToolkit.slug)}
          onChanged={() => void refreshComposio()}
          onClose={() => setActiveToolkit(null)}
        />
      )}
    </div>
  );
};

export default SkillsStep;
