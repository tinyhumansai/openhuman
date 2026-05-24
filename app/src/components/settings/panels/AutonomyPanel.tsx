import { useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  openhumanGetAutonomySettings,
  openhumanUpdateAutonomySettings,
} from '../../../utils/tauriCommands/config';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

// u32::MAX — the Rust default and our sentinel for "no limit". Inputs at or
// above this value render as "Unlimited" and clamp to UNLIMITED on save.
const UNLIMITED = 4_294_967_295;

/** Preset rows. The `label` field is an i18n key for the unlimited entry; the
 *  numeric-only rows are intentionally locale-agnostic. */
const PRESETS: { labelKey?: string; label?: string; value: number }[] = [
  { labelKey: 'autonomy.presetUnlimited', value: UNLIMITED },
  { label: '100', value: 100 },
  { label: '500', value: 500 },
  { label: '1000', value: 1000 },
];

const MIN = 1;
const MAX = UNLIMITED;

type Status =
  | { kind: 'idle' }
  | { kind: 'loading' }
  | { kind: 'saving' }
  | { kind: 'saved' }
  | { kind: 'error'; message: string };

/**
 * Settings panel under Developer Options for editing the agent's
 * max_actions_per_hour rate-limit. Loads the current value via
 * openhumanGetAutonomySettings on mount; saving writes through
 * openhumanUpdateAutonomySettings and persists to the user's config.toml.
 * New value applies to the next agent session.
 */
const AutonomyPanel = () => {
  const { t } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const [committed, setCommitted] = useState<number | null>(null);
  const [draft, setDraft] = useState<string>('');
  const [status, setStatus] = useState<Status>({ kind: 'loading' });

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const res = await openhumanGetAutonomySettings();
        if (cancelled) return;
        const value = res.result.max_actions_per_hour;
        setCommitted(value);
        setDraft(String(value));
        setStatus({ kind: 'idle' });
      } catch (err) {
        if (cancelled) return;
        setStatus({ kind: 'error', message: err instanceof Error ? err.message : String(err) });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const trimmed = draft.trim();
  const parsed = Number(trimmed);
  const isValid =
    /^\d+$/.test(trimmed) && Number.isInteger(parsed) && parsed >= MIN && parsed <= MAX;
  const isChanged = committed !== null && parsed !== committed;
  const canSave = isValid && isChanged && status.kind !== 'saving';

  const applyPreset = (value: number) => {
    setDraft(String(value));
    if (status.kind === 'saved' || status.kind === 'error') {
      setStatus({ kind: 'idle' });
    }
  };

  const onSave = async () => {
    if (!canSave) return;
    setStatus({ kind: 'saving' });
    try {
      await openhumanUpdateAutonomySettings({ max_actions_per_hour: parsed });
      setCommitted(parsed);
      setStatus({ kind: 'saved' });
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      // Revert UI to last committed value, then surface the error.
      if (committed !== null) setDraft(String(committed));
      setStatus({ kind: 'error', message });
    }
  };

  return (
    <div className="z-10 relative">
      <SettingsHeader
        title={t('autonomy.title')}
        showBackButton
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />
      <div className="p-4 flex flex-col gap-4">
        <section className="px-4 py-3 rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900">
          <label
            htmlFor="autonomy-max-actions"
            className="block text-sm font-semibold text-stone-900 dark:text-neutral-100">
            {t('autonomy.maxActionsLabel')}
          </label>
          <p className="text-xs text-stone-600 dark:text-neutral-400 mt-1">
            {t('autonomy.maxActionsHelp')}
          </p>

          <div className="mt-3 flex items-center gap-2">
            <input
              id="autonomy-max-actions"
              type="number"
              min={MIN}
              max={MAX}
              step={1}
              value={draft}
              onChange={e => {
                setDraft(e.target.value);
                if (status.kind === 'saved' || status.kind === 'error') {
                  setStatus({ kind: 'idle' });
                }
              }}
              disabled={status.kind === 'loading' || status.kind === 'saving'}
              className="w-32 px-3 py-1.5 rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 text-sm font-mono"
            />
            <button
              onClick={onSave}
              disabled={!canSave}
              className="px-3 py-1.5 rounded-md bg-primary-600 hover:bg-primary-500 disabled:opacity-50 text-white text-xs font-medium transition-colors">
              {status.kind === 'saving' ? t('autonomy.statusSaving') : t('common.save')}
            </button>
          </div>

          <div className="mt-3 flex flex-wrap gap-2">
            {PRESETS.map(p => (
              <button
                key={p.value}
                onClick={() => applyPreset(p.value)}
                className="px-2 py-1 rounded-md border border-stone-200 dark:border-neutral-800 text-xs text-stone-700 dark:text-neutral-200 hover:bg-stone-100 dark:hover:bg-neutral-800">
                {p.labelKey ? t(p.labelKey) : p.label}
              </button>
            ))}
          </div>

          <div
            role="status"
            aria-live="polite"
            aria-atomic="true"
            className="mt-3 text-xs min-h-[1rem]">
            {!isValid && draft.trim() !== '' && (
              <span className="text-coral-600 dark:text-coral-300">
                {t('autonomy.invalidIntegerMsg')}
              </span>
            )}
            {isValid && parsed === UNLIMITED && (
              <span className="text-stone-500 dark:text-neutral-400">
                {t('autonomy.unlimitedNote')}
              </span>
            )}
            {status.kind === 'saved' && (
              <span className="text-sage-700 dark:text-sage-300">{t('autonomy.statusSaved')}</span>
            )}
            {status.kind === 'error' && (
              <span className="text-coral-600 dark:text-coral-300">
                {t('autonomy.statusFailed')}: {status.message}
              </span>
            )}
          </div>
        </section>
      </div>
    </div>
  );
};

export default AutonomyPanel;
