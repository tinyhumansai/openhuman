import debug from 'debug';
import { useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  PERSONA_FILE_SOUL,
  readPersonaFile,
  resetPersonaFile,
  writePersonaFile,
} from '../../../services/api/personaFilesApi';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import {
  MAX_PERSONA_DESCRIPTION_LEN,
  MAX_PERSONA_DISPLAY_NAME_LEN,
  selectPersonaDescription,
  selectPersonaDisplayName,
  setPersonaDescription,
  setPersonaDisplayName,
} from '../../../store/personaSlice';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const log = debug('persona:panel');

const PersonaPanel = () => {
  const { t } = useT();
  const { navigateBack, navigateToSettings, breadcrumbs } = useSettingsNavigation();
  const dispatch = useAppDispatch();

  const storedDisplayName = useAppSelector(selectPersonaDisplayName);
  const storedDescription = useAppSelector(selectPersonaDescription);

  const [nameDraft, setNameDraft] = useState(storedDisplayName);
  const [descriptionDraft, setDescriptionDraft] = useState(storedDescription);

  // Re-sync drafts when the store is reset externally (e.g. resetUserScopedState
  // during an identity flip) so Save can't write stale values into a clean store.
  useEffect(() => {
    setNameDraft(storedDisplayName);
  }, [storedDisplayName]);
  useEffect(() => {
    setDescriptionDraft(storedDescription);
  }, [storedDescription]);

  // SOUL.md editor state. The file is loaded over RPC on mount; `isDefault`
  // tracks whether the current on-disk copy is the bundled prompt so the UI can
  // disable Reset when there is nothing to restore.
  const [soulDraft, setSoulDraft] = useState('');
  const [soulSaved, setSoulSaved] = useState('');
  const [soulIsDefault, setSoulIsDefault] = useState(true);
  const [soulLoading, setSoulLoading] = useState(true);
  const [soulError, setSoulError] = useState<string | null>(null);
  const [soulBusy, setSoulBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    log('[ui-flow] soul.load:start file=%s', PERSONA_FILE_SOUL);
    readPersonaFile(PERSONA_FILE_SOUL)
      .then(file => {
        if (cancelled) return;
        setSoulDraft(file.contents);
        setSoulSaved(file.contents);
        setSoulIsDefault(file.is_default);
        setSoulError(null);
        log('[ui-flow] soul.load:ok is_default=%s', file.is_default);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        log('[ui-flow] soul.load:error %s', err instanceof Error ? err.message : err);
        setSoulError(err instanceof Error ? err.message : 'Could not load SOUL.md');
      })
      .finally(() => {
        if (!cancelled) setSoulLoading(false);
      });
    return () => {
      cancelled = true;
    };
    // Load once on mount — `t` is intentionally excluded so a locale change
    // does not re-fetch and overwrite unsaved edits.
  }, []);

  const nameDirty = nameDraft.trim() !== storedDisplayName;
  const descriptionDirty = descriptionDraft.trim() !== storedDescription;
  const identityDirty = nameDirty || descriptionDirty;

  const onSaveIdentity = () => {
    if (nameDirty) dispatch(setPersonaDisplayName(nameDraft));
    if (descriptionDirty) dispatch(setPersonaDescription(descriptionDraft));
  };

  const soulDirty = soulDraft !== soulSaved;

  const onSaveSoul = async () => {
    setSoulBusy(true);
    setSoulError(null);
    log('[ui-flow] soul.save:start bytes=%d', soulDraft.length);
    try {
      const file = await writePersonaFile(PERSONA_FILE_SOUL, soulDraft);
      setSoulDraft(file.contents);
      setSoulSaved(file.contents);
      setSoulIsDefault(file.is_default);
      log('[ui-flow] soul.save:ok');
    } catch (err) {
      log('[ui-flow] soul.save:error %s', err instanceof Error ? err.message : err);
      setSoulError(err instanceof Error ? err.message : t('settings.persona.soul.saveError'));
    } finally {
      setSoulBusy(false);
    }
  };

  const onResetSoul = async () => {
    setSoulBusy(true);
    setSoulError(null);
    log('[ui-flow] soul.reset:start');
    try {
      const file = await resetPersonaFile(PERSONA_FILE_SOUL);
      setSoulDraft(file.contents);
      setSoulSaved(file.contents);
      setSoulIsDefault(file.is_default);
      log('[ui-flow] soul.reset:ok');
    } catch (err) {
      log('[ui-flow] soul.reset:error %s', err instanceof Error ? err.message : err);
      setSoulError(err instanceof Error ? err.message : t('settings.persona.soul.resetError'));
    } finally {
      setSoulBusy(false);
    }
  };

  return (
    <div>
      <SettingsHeader
        title={t('settings.persona.title')}
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-4">
        {/* ── Identity ─────────────────────────────────────────────── */}
        <div>
          <h3 className="text-xs font-semibold uppercase tracking-wider text-stone-400 dark:text-neutral-500 mb-2 px-1">
            {t('settings.persona.identityHeading')}
          </h3>
          <div className="bg-white dark:bg-neutral-900 rounded-xl border border-stone-200 dark:border-neutral-800 p-4 space-y-3">
            <label className="block space-y-1">
              <span className="text-xs font-medium text-stone-600 dark:text-neutral-300">
                {t('settings.persona.displayNameLabel')}
              </span>
              <input
                aria-label={t('settings.persona.displayNameLabel')}
                data-testid="persona-display-name-input"
                value={nameDraft}
                maxLength={MAX_PERSONA_DISPLAY_NAME_LEN}
                placeholder={t('settings.persona.displayNamePlaceholder')}
                onChange={e => setNameDraft(e.target.value)}
                className="w-full rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:outline-none focus:ring-1 focus:ring-primary-400"
              />
            </label>
            <label className="block space-y-1">
              <span className="text-xs font-medium text-stone-600 dark:text-neutral-300">
                {t('settings.persona.descriptionLabel')}
              </span>
              <textarea
                aria-label={t('settings.persona.descriptionLabel')}
                data-testid="persona-description-input"
                value={descriptionDraft}
                maxLength={MAX_PERSONA_DESCRIPTION_LEN}
                rows={3}
                placeholder={t('settings.persona.descriptionPlaceholder')}
                onChange={e => setDescriptionDraft(e.target.value)}
                className="w-full resize-y rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:outline-none focus:ring-1 focus:ring-primary-400"
              />
            </label>
            <div className="flex justify-end">
              <button
                type="button"
                data-testid="persona-identity-save"
                onClick={onSaveIdentity}
                disabled={!identityDirty}
                className="px-3 py-1.5 text-xs rounded-md bg-primary-600 hover:bg-primary-700 disabled:opacity-60 text-white">
                {t('common.save')}
              </button>
            </div>
          </div>
          <p className="text-xs text-stone-500 dark:text-neutral-400 leading-relaxed px-1 mt-2">
            {t('settings.persona.identityDesc')}
          </p>
        </div>

        {/* ── Personality (SOUL.md) ────────────────────────────────── */}
        <div>
          <h3 className="text-xs font-semibold uppercase tracking-wider text-stone-400 dark:text-neutral-500 mb-2 px-1">
            {t('settings.persona.soul.heading')}
          </h3>
          <div className="bg-white dark:bg-neutral-900 rounded-xl border border-stone-200 dark:border-neutral-800 p-4 space-y-3">
            {soulLoading ? (
              <p className="text-sm text-stone-500 dark:text-neutral-400">{t('common.loading')}</p>
            ) : (
              <>
                <textarea
                  aria-label={t('settings.persona.soul.editorLabel')}
                  data-testid="persona-soul-editor"
                  value={soulDraft}
                  rows={12}
                  spellCheck={false}
                  onChange={e => setSoulDraft(e.target.value)}
                  className="w-full resize-y rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 font-mono text-xs leading-relaxed text-stone-900 dark:text-neutral-100 focus:outline-none focus:ring-1 focus:ring-primary-400"
                />
                <div className="flex flex-wrap items-center gap-2">
                  <button
                    type="button"
                    data-testid="persona-soul-save"
                    onClick={() => void onSaveSoul()}
                    disabled={soulBusy || !soulDirty}
                    className="px-3 py-1.5 text-xs rounded-md bg-primary-600 hover:bg-primary-700 disabled:opacity-60 text-white">
                    {t('common.save')}
                  </button>
                  <button
                    type="button"
                    data-testid="persona-soul-reset"
                    onClick={() => void onResetSoul()}
                    disabled={soulBusy || soulIsDefault}
                    className="px-3 py-1.5 text-xs rounded-md border border-stone-300 dark:border-neutral-700 hover:border-stone-400 dark:hover:border-neutral-600 disabled:opacity-60 text-stone-700 dark:text-neutral-200">
                    {t('settings.persona.soul.reset')}
                  </button>
                  {soulIsDefault && (
                    <span
                      data-testid="persona-soul-default-badge"
                      className="text-[11px] text-stone-500 dark:text-neutral-400">
                      {t('settings.persona.soul.usingDefault')}
                    </span>
                  )}
                </div>
              </>
            )}
            {soulError && (
              <p
                data-testid="persona-soul-error"
                className="text-xs text-coral-700 dark:text-coral-300">
                {soulError}
              </p>
            )}
          </div>
          <p className="text-xs text-stone-500 dark:text-neutral-400 leading-relaxed px-1 mt-2">
            {t('settings.persona.soul.desc')}
          </p>
        </div>

        {/* ── Appearance & Voice (handled in Mascot settings) ──────── */}
        <div>
          <h3 className="text-xs font-semibold uppercase tracking-wider text-stone-400 dark:text-neutral-500 mb-2 px-1">
            {t('settings.persona.appearanceHeading')}
          </h3>
          <div className="bg-white dark:bg-neutral-900 rounded-xl border border-stone-200 dark:border-neutral-800 p-4">
            <button
              type="button"
              data-testid="persona-open-mascot"
              onClick={() => navigateToSettings('mascot')}
              className="flex w-full items-center justify-between text-left text-sm text-stone-700 dark:text-neutral-200 hover:text-primary-700 dark:hover:text-primary-300">
              <span>{t('settings.persona.openMascotSettings')}</span>
              <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M9 5l7 7-7 7"
                />
              </svg>
            </button>
          </div>
          <p className="text-xs text-stone-500 dark:text-neutral-400 leading-relaxed px-1 mt-2">
            {t('settings.persona.appearanceDesc')}
          </p>
        </div>
      </div>
    </div>
  );
};

export default PersonaPanel;
