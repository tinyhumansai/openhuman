import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { callCoreRpc } from '../../../services/coreRpcClient';
import { CORE_RPC_METHODS } from '../../../services/rpcMethods';
import { useAppDispatch } from '../../../store/hooks';
import { setPrivacyMode } from '../../../store/privacySlice';
import { SettingsSection, SettingsStatusLine } from '../controls';

const log = debug('privacy-mode');

/** Privacy Mode values as serialized by the Rust core (snake_case). */
export type PrivacyMode = 'local_only' | 'standard' | 'sensitive';

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
  const dispatch = useAppDispatch();
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
        // Keep the Redux privacy slice (and therefore the persistent status
        // pill) in lock-step with the setting. Without this the pill shows the
        // stale mode until the next app reload — the provider only hydrates the
        // slice once on mount (#4437).
        dispatch(setPrivacyMode(resp.result.mode));
        setStatus('saved');
        setTimeout(() => setStatus('idle'), 2000);
      } catch (err) {
        console.warn('[privacy-mode] failed to set privacy mode:', err);
        setError(err instanceof Error ? err.message : String(err));
        setStatus('error');
      }
    },
    [mode, dispatch]
  );

  return (
    <SettingsSection title={t('privacy.mode.title')}>
      <div className="p-4 flex flex-col gap-3">
        <p className="text-xs text-content-muted leading-relaxed">
          {t('privacy.mode.description')}
        </p>
        <div className="flex flex-col gap-2" data-testid="privacy-mode-options">
          {MODES.map(({ value, labelKey, descKey }) => {
            const isSelected = mode === value;
            return (
              <button
                key={value}
                type="button"
                role="radio"
                aria-checked={isSelected}
                disabled={status === 'saving' || status === 'loading'}
                onClick={() => {
                  void handleSelect(value);
                }}
                data-testid={`privacy-mode-option-${value}`}
                className={`w-full text-left px-4 py-3 rounded-lg border transition-colors ${
                  isSelected
                    ? 'border-primary-500 bg-primary-50 dark:bg-primary-900/20'
                    : 'border-line bg-surface hover:border-line-strong dark:hover:border-line-strong'
                } ${status === 'saving' ? 'opacity-50' : ''}`}>
                <span className="text-sm font-semibold text-content">{t(labelKey)}</span>
                <p className="text-xs text-content-muted mt-0.5">{t(descKey)}</p>
              </button>
            );
          })}
        </div>
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
