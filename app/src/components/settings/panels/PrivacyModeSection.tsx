import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { cn } from '../../../lib/cn';
import { useT } from '../../../lib/i18n/I18nContext';
import { callCoreRpc } from '../../../services/coreRpcClient';
import { CORE_RPC_METHODS } from '../../../services/rpcMethods';
import { RadioGroupItem, RadioGroupRoot } from '../../ui/RadioGroup';
import { SettingsSection, SettingsStatusLine } from '../controls';

const log = debug('privacy-mode');

/** Privacy Mode values as serialized by the Rust core (snake_case). */
type PrivacyMode = 'local_only' | 'standard' | 'sensitive';

interface PrivacyModeResult {
  mode: PrivacyMode;
}

type Status = 'loading' | 'idle' | 'saving' | 'saved' | 'error';

const MODES: { value: PrivacyMode; labelKey: string; descKey: string }[] = [
  {
    value: 'local_only',
    labelKey: 'privacy.mode.localOnly',
    descKey: 'privacy.mode.localOnlyDesc',
  },
  { value: 'standard', labelKey: 'privacy.mode.standard', descKey: 'privacy.mode.standardDesc' },
  { value: 'sensitive', labelKey: 'privacy.mode.sensitive', descKey: 'privacy.mode.sensitiveDesc' },
];

/**
 * Privacy Mode selector (#4435). Reads and writes the data-egress posture
 * (local_only | standard | sensitive) via the core RPCs. Distinct from the
 * autonomy access mode. Rendered inside {@link PrivacyPanel}, but kept a
 * standalone component so it can be unit-tested without the CoreStateProvider.
 */
const PrivacyModeSection = () => {
  const { t } = useT();
  const [mode, setMode] = useState<PrivacyMode | null>(null);
  const [status, setStatus] = useState<Status>('loading');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    log('[privacy-mode] fetching current mode');
    callCoreRpc<{ result: PrivacyModeResult }>({
      method: CORE_RPC_METHODS.configGetPrivacyMode,
      params: {},
    })
      .then(resp => {
        if (cancelled) return;
        log('[privacy-mode] current mode', resp.result.mode);
        setMode(resp.result.mode);
        setStatus('idle');
      })
      .catch(err => {
        if (cancelled) return;
        console.warn('[privacy-mode] failed to load privacy mode:', err);
        setError(err instanceof Error ? err.message : String(err));
        setStatus('error');
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const handleSelect = useCallback(
    async (next: PrivacyMode) => {
      if (next === mode) return;
      log('[privacy-mode] setting mode', next);
      setStatus('saving');
      setError(null);
      try {
        const resp = await callCoreRpc<{ result: PrivacyModeResult }>({
          method: CORE_RPC_METHODS.configSetPrivacyMode,
          params: { mode: next },
        });
        setMode(resp.result.mode);
        setStatus('saved');
        setTimeout(() => setStatus('idle'), 2000);
      } catch (err) {
        console.warn('[privacy-mode] failed to set privacy mode:', err);
        setError(err instanceof Error ? err.message : String(err));
        setStatus('error');
      }
    },
    [mode]
  );

  return (
    <SettingsSection title={t('privacy.mode.title')}>
      <div className="p-4 flex flex-col gap-3">
        <p className="text-xs text-content-muted leading-relaxed">
          {t('privacy.mode.description')}
        </p>
        <RadioGroupRoot
          value={mode ?? undefined}
          onValueChange={next => void handleSelect(next as PrivacyMode)}
          aria-label={t('privacy.mode.title')}
          className="flex flex-col gap-2"
          data-testid="privacy-mode-options">
          {MODES.map(({ value, labelKey, descKey }) => {
            const isSelected = mode === value;
            const inputId = `privacy-mode-option-${value}-input`;
            return (
              <label
                key={value}
                htmlFor={inputId}
                className={cn(
                  'w-full text-left px-4 py-3 rounded-lg border transition-colors block',
                  isSelected
                    ? 'border-primary-500 bg-primary-50 dark:bg-primary-500/10'
                    : 'border-line bg-surface hover:border-line-strong',
                  status === 'saving' && 'opacity-50'
                )}>
                <RadioGroupItem
                  id={inputId}
                  value={value}
                  data-testid={`privacy-mode-option-${value}`}
                  disabled={status === 'saving' || status === 'loading'}
                  className="sr-only"
                />
                <span className="text-sm font-semibold text-content">{t(labelKey)}</span>
                <p className="text-xs text-content-muted mt-0.5">{t(descKey)}</p>
              </label>
            );
          })}
        </RadioGroupRoot>
        <SettingsStatusLine
          saving={status === 'saving'}
          savedNote={status === 'saved' ? t('privacy.mode.saved') : null}
          error={status === 'error' ? (error ?? t('privacy.mode.saveError')) : null}
          savingLabel={t('autonomy.statusSaving')}
        />
      </div>
    </SettingsSection>
  );
};

export default PrivacyModeSection;
