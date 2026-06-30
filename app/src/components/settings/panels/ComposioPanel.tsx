// [composio-direct] Settings panel for the Composio routing mode toggle
// (Backend / Direct BYO API key). Shipped in PR3 of #1710 — see
// `src/openhuman/composio/client.rs::create_composio_client` for the
// matching Rust factory.
//
// Why a separate panel from ComposioTriagePanel:
//   - ComposioTriagePanel governs the per-trigger LLM triage opt-out,
//     a behavior that lives entirely inside the backend-proxied
//     pipeline. Mixing the BYO-key controls into it would conflate two
//     orthogonal concerns and confuse users (triggers don't work at all
//     in direct mode — separately calling that out is cleaner).
//   - PR1 already owns LocalModelPanel and PR2 owns VoicePanel; this PR
//     stays in its lane by introducing a new file rather than editing
//     BackendProviderPanel / LocalModelPanel / VoicePanel.
import { useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { useCoreState } from '../../../providers/CoreStateProvider';
import { isLocalSessionToken } from '../../../utils/localSession';
import {
  type ComposioModeStatus,
  openhumanComposioClearApiKey,
  openhumanComposioGetMode,
  openhumanComposioSetApiKey,
} from '../../../utils/tauriCommands';
import PanelPage from '../../layout/PanelPage';
import Button from '../../ui/Button';
import SettingsBackButton from '../components/SettingsBackButton';
import { SettingsRow, SettingsSection, SettingsStatusLine, SettingsTextField } from '../controls';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

type Mode = 'backend' | 'direct';

interface ComposioPanelProps {
  /** When true, render without the SettingsHeader chrome (used when embedded
   *  inside the onboarding custom wizard). */
  embedded?: boolean;
  /** Whether OpenHuman-managed auth should be offered. Defaults to true for
   *  cloud-authenticated sessions and false otherwise. */
  managedAuthEnabled?: boolean;
}

const ComposioPanel = ({ embedded = false, managedAuthEnabled }: ComposioPanelProps = {}) => {
  const { t } = useT();
  const { navigateBack } = useSettingsNavigation();
  const { snapshot } = useCoreState();
  const allowManagedAuth =
    managedAuthEnabled ??
    (Boolean(snapshot.sessionToken) && !isLocalSessionToken(snapshot.sessionToken));

  const [mode, setMode] = useState<Mode>('backend');
  // Tracks the mode that's actually persisted on disk — distinct from
  // the in-flight `mode` radio selection so we can tell whether a Save
  // click constitutes a Backend → Direct *transition* (which needs a
  // confirmation gate) vs. just persisting a new API key while already
  // in Direct mode.
  const [persistedMode, setPersistedMode] = useState<Mode>('backend');
  const [apiKey, setApiKey] = useState('');
  const [apiKeyStored, setApiKeyStored] = useState(false);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [saveStatus, setSaveStatus] = useState<'idle' | 'saved' | 'error' | 'cleared'>('idle');
  // Confirmation gate for the Backend → Direct transition. The state
  // machine has two arms: `idle` (Save acts immediately) and
  // `awaiting` (Save was clicked while transitioning Backend → Direct
  // with a fresh key — the user sees the warning copy and must hit
  // "I understand, switch to Direct" or "Cancel"). Direct → Backend
  // doesn't need this because that recovery is reversible (re-paste
  // the key to flip back).
  const [confirmGate, setConfirmGate] = useState<'idle' | 'awaiting'>('idle');
  const saveStatusTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // ── load current mode status on mount ────────────────────────────
  useEffect(() => {
    let isMounted = true;
    openhumanComposioGetMode()
      .then(res => {
        if (!isMounted) return;
        const status: ComposioModeStatus | undefined = res.result;
        if (!status) return;
        const normalizedMode: Mode =
          !allowManagedAuth || status.mode === 'direct' ? 'direct' : 'backend';
        setMode(normalizedMode);
        setPersistedMode(normalizedMode);
        setApiKeyStored(Boolean(status.api_key_set));
      })
      .catch(err => {
        if (!isMounted) return;
        // [composio-direct] never re-throw — settings panel should
        // still render so the user can recover by toggling manually.
        console.warn('[ComposioPanel] failed to load mode:', err);
      })
      .finally(() => {
        if (isMounted) setLoading(false);
      });

    return () => {
      isMounted = false;
      if (saveStatusTimer.current !== null) {
        clearTimeout(saveStatusTimer.current);
      }
    };
  }, [allowManagedAuth]);

  const flashSaved = (status: 'saved' | 'cleared') => {
    setSaveStatus(status);
    if (saveStatusTimer.current !== null) {
      clearTimeout(saveStatusTimer.current);
    }
    saveStatusTimer.current = setTimeout(() => setSaveStatus('idle'), 3000);
    // [composio-cache] Notify in-renderer subscribers (notably
    // useComposioIntegrations) that the routing config just changed so
    // they can drop their cached connection / toolkit state and
    // re-fetch against the new client. Mirrors the core-side
    // DomainEvent::ComposioConfigChanged emitted by the matching RPC
    // op. Without this the integrations panel keeps showing the
    // previous tenant's badge for up to one poll interval (5s).
    try {
      window.dispatchEvent(new CustomEvent('composio:config-changed'));
    } catch (err) {
      // Non-fatal — old browsers without CustomEvent support shouldn't
      // crash the panel.
      console.warn('[composio-cache] dispatch composio:config-changed failed:', err);
    }
  };

  // Indicates this Save click would transition the persisted mode from
  // Backend to Direct *with* a freshly-pasted key. We gate this exact
  // transition on a confirmation step because the consequences are not
  // obvious from the radio toggle alone — the user's previously-linked
  // integrations (Gmail, Slack, GitHub, …) live in TinyHumans' Composio
  // tenant and will simply disappear from the integrations panel until
  // they re-link them through their personal app.composio.dev account.
  const isBackendToDirectTransition = (): boolean => {
    const trimmed = apiKey.trim();
    return persistedMode === 'backend' && mode === 'direct' && trimmed.length > 0;
  };

  const performSave = async () => {
    const trimmed = apiKey.trim();
    setSaving(true);
    try {
      if (mode === 'direct' && trimmed.length > 0) {
        // [composio-direct] persist new key + flip mode to direct.
        await openhumanComposioSetApiKey(trimmed, true);
        // Mask the field after a successful save so the secret is not
        // left dangling in the DOM. The Rust side has the source of
        // truth in the encrypted keychain.
        setApiKey('');
        setApiKeyStored(true);
        setPersistedMode('direct');
        flashSaved('saved');
      } else if (mode === 'backend') {
        // Switching to backend — clear the stored key and reset mode.
        await openhumanComposioClearApiKey();
        setApiKey('');
        setApiKeyStored(false);
        setPersistedMode('backend');
        flashSaved('cleared');
      } else {
        // Direct selected, no new key but one already stored — nothing
        // to persist; just acknowledge.
        flashSaved('saved');
      }
    } catch (err) {
      console.warn('[ComposioPanel] failed to save:', err);
      if (saveStatusTimer.current !== null) {
        clearTimeout(saveStatusTimer.current);
        saveStatusTimer.current = null;
      }
      setSaveStatus('error');
    } finally {
      setSaving(false);
      setConfirmGate('idle');
    }
  };

  const handleSave = async () => {
    const trimmed = apiKey.trim();
    if (mode === 'direct' && trimmed.length === 0 && !apiKeyStored) {
      // Direct mode without a key is a no-op — flag it clearly instead
      // of round-tripping to the backend just to get an error string.
      setSaveStatus('error');
      return;
    }
    if (isBackendToDirectTransition()) {
      // [composio-direct] Show the confirmation step instead of saving
      // straight away. The user-visible consequences (existing
      // integrations disappear, triggers don't fire) aren't obvious
      // from the radio toggle alone.
      console.debug('[composio-direct] Backend → Direct transition pending user confirmation');
      setConfirmGate('awaiting');
      return;
    }
    await performSave();
  };

  const handleConfirmTransition = async () => {
    console.debug('[composio-direct] Backend → Direct transition confirmed by user');
    await performSave();
  };

  const handleCancelTransition = () => {
    console.debug('[composio-direct] Backend → Direct transition cancelled by user');
    setConfirmGate('idle');
  };

  const composioDescription = embedded
    ? undefined
    : t('settings.developerMenu.composioRouting.desc');
  const composioLeading = embedded ? undefined : <SettingsBackButton onBack={navigateBack} />;

  if (loading) {
    return (
      <PanelPage contentClassName="" description={composioDescription} leading={composioLeading}>
        <div className={embedded ? '' : 'p-4'}>
          <p className="text-sm text-content-muted">{t('settings.composio.loading')}</p>
        </div>
      </PanelPage>
    );
  }

  return (
    <PanelPage
      className="z-10"
      contentClassName=""
      description={composioDescription}
      leading={composioLeading}>
      <div className={embedded ? 'space-y-5' : 'p-4 pt-2 space-y-5'}>
        <p className="text-sm text-content-muted">{t('settings.composio.intro')}</p>

        {allowManagedAuth ? (
          <SettingsSection>
            <fieldset className="px-4 py-3">
              <legend className="text-sm font-medium text-content mb-2">
                {t('settings.composio.routingMode')}
              </legend>
              <div className="space-y-2">
                <label className="flex items-start gap-3 cursor-pointer">
                  <input
                    type="radio"
                    name="composio-mode"
                    value="backend"
                    checked={mode === 'backend'}
                    onChange={() => setMode('backend')}
                    aria-label={t('settings.composio.modeManaged')}
                    className="mt-1"
                  />
                  <div className="text-left">
                    <span className="text-sm font-medium text-content">
                      {t('settings.composio.modeManaged')}
                    </span>
                    <p className="text-xs text-content-muted mt-0.5">
                      {t('settings.composio.modeManagedDesc')}
                    </p>
                  </div>
                </label>
                <label className="flex items-start gap-3 cursor-pointer">
                  <input
                    type="radio"
                    name="composio-mode"
                    value="direct"
                    checked={mode === 'direct'}
                    onChange={() => setMode('direct')}
                    aria-label={t('settings.composio.modeDirect')}
                    className="mt-1"
                  />
                  <div className="text-left">
                    <span className="text-sm font-medium text-content">
                      {t('settings.composio.modeDirect')}
                    </span>
                    <p className="text-xs text-content-muted mt-0.5">
                      {t('settings.composio.modeDirectDesc')}
                    </p>
                  </div>
                </label>
              </div>
            </fieldset>
          </SettingsSection>
        ) : (
          <SettingsSection>
            <div className="px-4 py-3 space-y-2">
              <p className="text-sm font-medium text-content">
                {t('settings.composio.modeDirect')}
              </p>
              <p className="text-xs text-content-muted">
                {t(
                  'settings.composio.directOnlyDesc',
                  'Managed Composio auth is unavailable here. Enter your own Composio API key or skip this for now.'
                )}
              </p>
            </div>
          </SettingsSection>
        )}

        {/* API key field — only when Direct is selected */}
        {mode === 'direct' && (
          <SettingsSection
            title={t('settings.composio.apiKeyLabel')}
            description={t('settings.composio.apiKeyDesc')}>
            <SettingsRow
              stacked
              control={
                <div className="space-y-1">
                  <SettingsTextField
                    id="composio-api-key"
                    type="password"
                    autoComplete="off"
                    value={apiKey}
                    onChange={e => setApiKey(e.target.value)}
                    placeholder={
                      apiKeyStored
                        ? t('settings.composio.apiKeyStoredPlaceholder')
                        : t('settings.composio.apiKeyExamplePlaceholder')
                    }
                    aria-label={t('settings.composio.apiKeyLabel')}
                    mono
                  />
                  {apiKeyStored && (
                    <p className="text-xs text-sage-700 dark:text-sage-300">
                      {t('settings.composio.apiKeyStored')}
                    </p>
                  )}
                </div>
              }
            />
          </SettingsSection>
        )}

        {confirmGate === 'awaiting' ? (
          // [composio-direct] Inline confirmation step — kept as a
          // sibling state rather than a portal modal so the warning
          // copy stays in the same scroll context as the toggle the
          // user just changed. Easier to dismiss with the keyboard and
          // composes more naturally with the existing settings panel
          // chrome.
          <div
            role="alertdialog"
            aria-labelledby="composio-confirm-title"
            className="rounded-xl border border-amber-200 dark:border-amber-500/30 bg-amber-50/80 p-4 space-y-3">
            <p id="composio-confirm-title" className="text-sm font-medium text-amber-900">
              {t('settings.composio.confirmTitle')}
            </p>
            <div className="text-xs text-amber-900 space-y-2">
              <p>{t('settings.composio.confirmWarning')}</p>
              <p>{t('settings.composio.confirmNeedItems')}</p>
              <ol className="list-decimal list-inside space-y-0.5 ml-2">
                <li>{t('settings.composio.confirmItem1')}</li>
                <li>{t('settings.composio.confirmItem2')}</li>
                <li>{t('settings.composio.confirmItem3')}</li>
              </ol>
            </div>
            <div className="flex gap-2 pt-1">
              <Button
                type="button"
                variant="secondary"
                size="sm"
                onClick={handleCancelTransition}
                disabled={saving}
                className="flex-1">
                {t('common.cancel')}
              </Button>
              <Button
                type="button"
                size="sm"
                onClick={() => void handleConfirmTransition()}
                disabled={saving}
                className="flex-1 bg-amber-600 hover:bg-amber-500">
                {saving ? t('settings.composio.switching') : t('settings.composio.confirmSwitch')}
              </Button>
            </div>
          </div>
        ) : (
          <div className="flex items-center gap-3">
            <Button
              type="button"
              variant="primary"
              size="sm"
              onClick={() => void handleSave()}
              disabled={saving}>
              {saving ? t('settings.composio.saving') : t('common.save')}
            </Button>
            <SettingsStatusLine
              saving={false}
              savedNote={
                saveStatus === 'saved'
                  ? t('composio.settingsSaved')
                  : saveStatus === 'cleared'
                    ? t('settings.composio.clearedToBackend')
                    : null
              }
              error={saveStatus === 'error' ? t('settings.composio.saveErrorNoKey') : null}
              savingLabel=""
            />
          </div>
        )}
      </div>
    </PanelPage>
  );
};

export default ComposioPanel;
