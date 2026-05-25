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
    case 'expired':
      return 'bg-coral-500';
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
    case 'expired':
      return t('composio.authExpired');
    case 'error':
      return t('common.error');
    default:
      return '';
  }
}

function statusColor(state: ReturnType<typeof deriveComposioState>): string {
  switch (state) {
    case 'connected':
      return 'text-sage-600 dark:text-sage-300';
    case 'pending':
      return 'text-amber-600 dark:text-amber-300';
    case 'expired':
      return 'text-coral-600 dark:text-coral-300';
    case 'error':
      return 'text-coral-600 dark:text-coral-300';
    default:
      return 'text-stone-400 dark:text-neutral-500';
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
      setError(e instanceof Error ? e.message : t('bootCheck.actionFailed'));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="rounded-2xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-8 shadow-soft animate-fade-up">
      <div className="text-center mb-4">
        <h1 className="text-xl font-bold mb-2 text-stone-900 dark:text-neutral-100">
          {t('skills.connect')}
        </h1>
        <p className="text-stone-600 dark:text-neutral-300 text-sm">{t('skills.available')}</p>
      </div>

      <div className="mb-4 space-y-2">
        {composioError ? (
          <div className="rounded-xl border border-amber-200 dark:border-amber-500/30 bg-amber-50 dark:bg-amber-500/10 p-4 text-center">
            <p className="text-sm text-amber-700 dark:text-amber-300 mb-2">{t('common.error')}</p>
            <button
              type="button"
              onClick={() => void refreshComposio()}
              className="text-xs font-medium text-amber-800 border border-amber-300 rounded-lg px-3 py-1 hover:bg-amber-100 dark:bg-amber-500/20 transition-colors">
              {t('common.retry')}
            </button>
          </div>
        ) : (
          <button
            type="button"
            onClick={() => setActiveToolkit(gmailMeta)}
            data-testid="onboarding-skills-gmail-button"
            className="w-full flex items-center gap-3 rounded-xl border border-stone-100 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-3 transition-colors hover:bg-stone-50 dark:hover:bg-neutral-800/60 text-left">
            <div className="flex h-8 w-8 flex-shrink-0 items-center justify-center text-lg">
              {gmailMeta.icon}
            </div>

            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                <span className="truncate text-sm font-semibold text-stone-900 dark:text-neutral-100">
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
              <p className="mt-0.5 line-clamp-1 text-xs leading-relaxed text-stone-500 dark:text-neutral-400">
                {gmailMeta.description}
              </p>
            </div>

            <span
              className={`flex-shrink-0 rounded-lg border px-3 py-1.5 text-[11px] font-medium transition-colors ${
                gmailConnected
                  ? 'border-sage-200 dark:border-sage-500/30 bg-sage-50 dark:bg-sage-500/10 text-sage-700 dark:text-sage-300'
                  : gmailState === 'pending'
                    ? 'border-amber-200 dark:border-amber-500/30 bg-amber-50 dark:bg-amber-500/10 text-amber-700 dark:text-amber-300'
                    : gmailState === 'expired'
                      ? 'border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 text-coral-700 dark:text-coral-300'
                      : 'border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/15 text-primary-700 dark:text-primary-300'
              }`}>
              {gmailConnected
                ? t('skills.configure')
                : gmailState === 'expired'
                  ? t('composio.reconnect')
                  : t('skills.connect')}
            </span>
          </button>
        )}

        <div className="rounded-xl border border-stone-100 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3 py-2.5 text-center">
          <p className="text-xs text-stone-400 dark:text-neutral-500">{t('skills.available')}</p>
        </div>
      </div>

      {error && <p className="text-coral-400 text-sm mb-3 text-center">{error}</p>}

      <OnboardingNextButton
        onClick={handleContinue}
        loading={submitting}
        loadingLabel={t('common.loading')}
        label={gmailConnected ? t('common.continue') : t('onboarding.skipForNow')}
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
