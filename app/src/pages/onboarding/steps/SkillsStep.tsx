import { useState } from 'react';

import ComposioConnectModal from '../../../components/composio/ComposioConnectModal';
import {
  composioToolkitMeta,
  type ComposioToolkitMeta,
} from '../../../components/composio/toolkitMeta';
import { useComposioIntegrations } from '../../../lib/composio/hooks';
import { type ComposioConnection, deriveComposioState } from '../../../lib/composio/types';
import { useT } from '../../../lib/i18n/I18nContext';
import OnboardingNextButton from '../components/OnboardingNextButton';

export interface SkillsConnections {
  /** Wire-format source ids (e.g. `composio:gmail`). */
  sources: string[];
}

interface SkillsStepProps {
  onNext: (connections: SkillsConnections) => void | Promise<void>;
  onBack?: () => void;
}

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

function statusLabel(
  state: ReturnType<typeof deriveComposioState>,
  t: (key: string) => string
): string {
  switch (state) {
    case 'connected':
      return t('skills.connected');
    case 'pending':
      return t('channels.status.connecting');
    case 'error':
      return t('common.error');
    default:
      return '';
  }
}

function statusColor(state: ReturnType<typeof deriveComposioState>): string {
  switch (state) {
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
  const { t } = useT();
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [activeToolkit, setActiveToolkit] = useState<ComposioToolkitMeta | null>(null);

  const {
    connectionByToolkit,
    error: composioError,
    refresh: refreshComposio,
  } = useComposioIntegrations();

  const gmailMeta = composioToolkitMeta('gmail');
  const gmailConnection = connectionByToolkit.get('gmail');
  const gmailState = deriveComposioState(gmailConnection);
  const gmailConnected = gmailState === 'connected';

  const handleContinue = async () => {
    setError(null);
    setSubmitting(true);
    try {
      const sources = gmailConnected ? ['composio:gmail'] : [];
      await onNext({ sources });
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Something went wrong. Please try again.');
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="rounded-2xl border border-stone-200 bg-white p-8 shadow-soft animate-fade-up">
      <div className="text-center mb-4">
        <h1 className="text-xl font-bold mb-2 text-stone-900">{t('skills.connect')}</h1>
        <p className="text-stone-600 text-sm">{t('skills.available')}</p>
      </div>

      <div className="mb-4 space-y-2">
        {composioError ? (
          <div className="rounded-xl border border-amber-200 bg-amber-50 p-4 text-center">
            <p className="text-sm text-amber-700 mb-2">{t('common.error')}</p>
            <button
              type="button"
              onClick={() => void refreshComposio()}
              className="text-xs font-medium text-amber-800 border border-amber-300 rounded-lg px-3 py-1 hover:bg-amber-100 transition-colors">
              {t('common.retry')}
            </button>
          </div>
        ) : (
          <button
            type="button"
            onClick={() => setActiveToolkit(gmailMeta)}
            data-testid="onboarding-skills-gmail-button"
            className="w-full flex items-center gap-3 rounded-xl border border-stone-100 bg-white p-3 transition-colors hover:bg-stone-50 text-left">
            <div className="flex h-8 w-8 flex-shrink-0 items-center justify-center text-lg">
              {gmailMeta.icon}
            </div>

            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                <span className="truncate text-sm font-semibold text-stone-900">
                  {gmailMeta.name}
                </span>
                {statusLabel(gmailState, t) && (
                  <>
                    <div
                      className={`h-1.5 w-1.5 flex-shrink-0 rounded-full ${statusDotClass(gmailConnection)}`}
                    />
                    <span className={`flex-shrink-0 text-xs ${statusColor(gmailState)}`}>
                      {statusLabel(gmailState, t)}
                    </span>
                  </>
                )}
              </div>
              <p className="mt-0.5 line-clamp-1 text-xs leading-relaxed text-stone-500">
                {gmailMeta.description}
              </p>
            </div>

            <span
              className={`flex-shrink-0 rounded-lg border px-3 py-1.5 text-[11px] font-medium transition-colors ${
                gmailConnected
                  ? 'border-sage-200 bg-sage-50 text-sage-700'
                  : gmailState === 'pending'
                    ? 'border-amber-200 bg-amber-50 text-amber-700'
                    : 'border-primary-200 bg-primary-50 text-primary-700'
              }`}>
              {gmailConnected ? t('skills.configure') : t('skills.connect')}
            </span>
          </button>
        )}

        <div className="rounded-xl border border-stone-100 bg-stone-50 px-3 py-2.5 text-center">
          <p className="text-xs text-stone-400">{t('skills.available')}</p>
        </div>
      </div>

      {error && <p className="text-coral-400 text-sm mb-3 text-center">{error}</p>}

      <OnboardingNextButton
        onClick={handleContinue}
        loading={submitting}
        loadingLabel={t('common.loading')}
        label={gmailConnected ? t('common.continue') : 'Skip for Now'}
      />

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
