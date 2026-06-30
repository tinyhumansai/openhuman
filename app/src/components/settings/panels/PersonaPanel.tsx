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
import Button from '../../ui/Button';
import { SettingsRow, SettingsSection, SettingsTextArea, SettingsTextField } from '../controls';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import SettingsPanel from '../layout/SettingsPanel';

const log = debug('persona:panel');

interface PersonaPanelProps {
  /** When true the panel is hosted inside another settings page (the
   *  Personality & Face tabs) — skip the standalone SettingsHeader chrome. */
  embedded?: boolean;
}

const PersonaPanel = ({ embedded = false }: PersonaPanelProps) => {
  const { t } = useT();
  const { navigateToSettings } = useSettingsNavigation();
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

  const body = (
    <>
      {/* ── Identity ─────────────────────────────────────────────── */}
      <SettingsSection title={t('settings.persona.identityHeading')}>
        <SettingsRow
          htmlFor="persona-display-name"
          label={t('settings.persona.displayNameLabel')}
          stacked
          control={
            <SettingsTextField
              id="persona-display-name"
              aria-label={t('settings.persona.displayNameLabel')}
              data-testid="persona-display-name-input"
              value={nameDraft}
              maxLength={MAX_PERSONA_DISPLAY_NAME_LEN}
              placeholder={t('settings.persona.displayNamePlaceholder')}
              onChange={e => setNameDraft(e.target.value)}
            />
          }
        />
        <SettingsRow
          htmlFor="persona-description"
          label={t('settings.persona.descriptionLabel')}
          stacked
          control={
            <SettingsTextArea
              id="persona-description"
              aria-label={t('settings.persona.descriptionLabel')}
              data-testid="persona-description-input"
              value={descriptionDraft}
              maxLength={MAX_PERSONA_DESCRIPTION_LEN}
              rows={3}
              placeholder={t('settings.persona.descriptionPlaceholder')}
              onChange={e => setDescriptionDraft(e.target.value)}
            />
          }
        />
        <div className="flex justify-end px-4 py-3">
          <Button
            type="button"
            data-testid="persona-identity-save"
            variant="primary"
            size="xs"
            onClick={onSaveIdentity}
            disabled={!identityDirty}>
            {t('common.save')}
          </Button>
        </div>
      </SettingsSection>
      <p className="text-xs text-content-muted leading-relaxed px-1">
        {t('settings.persona.identityDesc')}
      </p>

      {/* ── Personality (SOUL.md) ────────────────────────────────── */}
      <SettingsSection title={t('settings.persona.soul.heading')}>
        {soulLoading ? (
          <div className="px-4 py-3">
            <p className="text-sm text-content-muted">{t('common.loading')}</p>
          </div>
        ) : (
          <>
            <div className="px-4 py-3">
              <SettingsTextArea
                aria-label={t('settings.persona.soul.editorLabel')}
                data-testid="persona-soul-editor"
                value={soulDraft}
                rows={12}
                spellCheck={false}
                className="font-mono text-xs leading-relaxed"
                onChange={e => setSoulDraft(e.target.value)}
              />
            </div>
            <div className="flex flex-wrap items-center gap-2 px-4 pb-3">
              <Button
                type="button"
                data-testid="persona-soul-save"
                variant="primary"
                size="xs"
                onClick={() => void onSaveSoul()}
                disabled={soulBusy || !soulDirty}>
                {t('common.save')}
              </Button>
              <Button
                type="button"
                data-testid="persona-soul-reset"
                variant="secondary"
                size="xs"
                onClick={() => void onResetSoul()}
                disabled={soulBusy || soulIsDefault}>
                {t('settings.persona.soul.reset')}
              </Button>
              {soulIsDefault && (
                <span
                  data-testid="persona-soul-default-badge"
                  className="text-[11px] text-content-muted">
                  {t('settings.persona.soul.usingDefault')}
                </span>
              )}
            </div>
          </>
        )}
        {soulError && (
          <p
            data-testid="persona-soul-error"
            className="px-4 pb-3 text-xs text-coral-700 dark:text-coral-300">
            {soulError}
          </p>
        )}
      </SettingsSection>
      <p className="text-xs text-content-muted leading-relaxed px-1">
        {t('settings.persona.soul.desc')}
      </p>

      {/* ── Appearance & Voice (handled in Mascot settings) ──────── */}
      <SettingsSection title={t('settings.persona.appearanceHeading')}>
        <div className="px-4 py-3">
          <button
            type="button"
            data-testid="persona-open-mascot"
            onClick={() => navigateToSettings('personality#face')}
            className="flex w-full items-center justify-between text-left text-sm text-content hover:text-primary-700 dark:hover:text-primary-300">
            <span>{t('settings.persona.openMascotSettings')}</span>
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
            </svg>
          </button>
        </div>
      </SettingsSection>
      <p className="text-xs text-content-muted leading-relaxed px-1">
        {t('settings.persona.appearanceDesc')}
      </p>
    </>
  );

  // Embedded inside the tabbed Personality & Face page: the parent owns the
  // header, so render just the padded body.
  if (embedded) return <div className="p-4 pt-2 space-y-5">{body}</div>;

  return <SettingsPanel>{body}</SettingsPanel>;
};

export default PersonaPanel;
