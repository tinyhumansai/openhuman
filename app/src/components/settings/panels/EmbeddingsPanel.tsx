/**
 * Embeddings settings panel — provider selection, API keys, model + dimensions.
 *
 * Flow: select a provider → if it needs an API key, a setup popup appears
 * to enter the key, test connection, and save. Dimension changes show a
 * destructive confirm dialog since they invalidate stored vectors.
 */
import createDebug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { useCoreState } from '../../../providers/CoreStateProvider';
import {
  clearEmbeddingsApiKey,
  type EmbeddingProviderEntry,
  type EmbeddingsSettings,
  type EmbeddingsTestResult,
  loadEmbeddingsSettings,
  setEmbeddingsApiKey,
  testEmbeddingsConnection,
  updateEmbeddingsSettings,
} from '../../../services/api/embeddingsApi';
import { isLocalSessionToken } from '../../../utils/localSession';
import PanelPage from '../../layout/PanelPage';
import { Alert, AlertDescription, Button, CenteredLoadingState, ConfirmDialog } from '../../ui';
import SettingsBackButton from '../components/SettingsBackButton';
import { SettingsStatusLine } from '../controls';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import EmbeddingsModelSection from './EmbeddingsModelSection';
import EmbeddingsProviderList from './EmbeddingsProviderList';
import EmbeddingsSetupModal from './EmbeddingsSetupModal';

// Grep-friendly, namespaced diagnostics for the custom-endpoint verification
// flow. Logs only safe metadata (error classification code, state transitions) —
// never the endpoint URL, API key, or backend-provided detail body.
const log = createDebug('app:settings:embeddings');

type Status =
  | { kind: 'idle' }
  | { kind: 'loading' }
  | { kind: 'saving' }
  | { kind: 'saved' }
  | { kind: 'error'; message: string };

function isBackendSessionError(message: string | undefined): boolean {
  const text = message ?? '';
  return (
    /no backend session/i.test(text) ||
    /SESSION_EXPIRED/i.test(text) ||
    /session expired/i.test(text) ||
    (/invalid token/i.test(text) && /(401|unauthorized)/i.test(text))
  );
}

interface EmbeddingsPanelProps {
  embedded?: boolean;
}

const EmbeddingsPanel = ({ embedded = false }: EmbeddingsPanelProps = {}) => {
  const { t } = useT();
  const { navigateBack } = useSettingsNavigation();
  const { snapshot, clearSession } = useCoreState();
  const isLocalSession = isLocalSessionToken(snapshot.sessionToken);

  const [settings, setSettings] = useState<EmbeddingsSettings | null>(null);
  const [status, setStatus] = useState<Status>({ kind: 'loading' });
  const [managedSessionMissing, setManagedSessionMissing] = useState(false);

  // Setup popup state
  const [setupProvider, setSetupProvider] = useState<EmbeddingProviderEntry | null>(null);
  const [setupKey, setSetupKey] = useState('');
  const [setupShowKey, setSetupShowKey] = useState(false);
  const [setupTesting, setSetupTesting] = useState(false);
  const [setupTestResult, setSetupTestResult] = useState<EmbeddingsTestResult | null>(null);
  const [setupSaving, setSetupSaving] = useState(false);
  const [setupError, setSetupError] = useState('');

  // Confirm wipe dialog
  const [pendingWipe, setPendingWipe] = useState<{
    provider?: string;
    model?: string;
    dimensions?: number;
    custom_endpoint?: string;
  } | null>(null);

  // Custom endpoint state
  const [customEndpoint, setCustomEndpoint] = useState('');
  const [customModel, setCustomModel] = useState('');
  const [customDims, setCustomDims] = useState('1024');

  const reload = useCallback(async () => {
    try {
      const s = await loadEmbeddingsSettings();
      setSettings(s);
      setStatus({ kind: 'idle' });
    } catch (err) {
      setStatus({ kind: 'error', message: err instanceof Error ? err.message : String(err) });
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  if (!settings) {
    return (
      <PanelPage
        className="z-10"
        contentClassName=""
        description={embedded ? undefined : t('pages.settings.ai.embeddingsDesc')}
        leading={embedded ? undefined : <SettingsBackButton onBack={navigateBack} />}>
        <div className={embedded ? '' : 'p-4'}>
          {status.kind === 'loading' ? (
            <CenteredLoadingState label={t('common.loading')} />
          ) : status.kind === 'error' ? (
            <Alert variant="destructive" density="compact">
              <AlertDescription>{status.message}</AlertDescription>
            </Alert>
          ) : null}
        </div>
      </PanelPage>
    );
  }

  const selectedProvider = normalizeProvider(settings.provider);
  const currentEntry = settings.providers.find(p => p.slug === selectedProvider);
  const currentModels = currentEntry?.models ?? [];
  const currentModel = currentModels.find(m => m.id === settings.model) ?? currentModels[0];
  const allowedDims = currentModel?.allowed_dimensions ?? [];
  const managedLoginMessage = t('settings.embeddings.managedLoginRequired');
  const managedRequiresLogin = isLocalSession && selectedProvider === 'managed';
  const showManagedLoginPrompt =
    (selectedProvider === 'managed' && (managedRequiresLogin || managedSessionMissing)) ||
    (isLocalSession && managedSessionMissing);

  function handleProviderClick(entry: EmbeddingProviderEntry) {
    if (entry.slug !== 'managed') setManagedSessionMissing(false);

    if (entry.slug === 'custom') {
      // Custom doubles as an editable saved profile. It must reopen even when
      // already selected, and it must hydrate from the retained profile when
      // embeddings are currently disabled or routed elsewhere.
      const loadedSettings = settings!;
      const activeEndpoint = loadedSettings.provider.startsWith('custom:')
        ? loadedSettings.provider.slice('custom:'.length)
        : '';
      const profile = activeEndpoint
        ? {
            endpoint: activeEndpoint,
            model: loadedSettings.model,
            dimensions: loadedSettings.dimensions,
          }
        : loadedSettings.custom_settings;
      setCustomEndpoint(profile?.endpoint ?? customEndpoint);
      setCustomModel(profile?.model ?? customModel);
      setCustomDims(String(profile?.dimensions ?? customDims));
      setSetupProvider(entry);
      setSetupKey('');
      setSetupShowKey(false);
      setSetupTestResult(null);
      setSetupError('');
      return;
    }

    if (entry.slug === selectedProvider) return;

    if (entry.slug === 'managed' && isLocalSession) {
      setManagedSessionMissing(true);
      setStatus({ kind: 'error', message: managedLoginMessage });
      return;
    }

    if (entry.requires_api_key && !entry.has_api_key) {
      // Open the setup popup for API key entry + test
      setSetupProvider(entry);
      setSetupKey('');
      setSetupShowKey(false);
      setSetupTestResult(null);
      setSetupError('');
      return;
    }

    // No key needed or already configured — switch directly
    void doProviderSwitch(entry.slug);
  }

  async function doProviderSwitch(slug: string, model?: string, dims?: number) {
    const entry = settings!.providers.find(p => p.slug === slug);
    const defaultModel = entry?.models[0];
    const newModel = model ?? defaultModel?.id ?? settings!.model;
    const newDims = dims ?? defaultModel?.default_dimensions ?? settings!.dimensions;

    if (slug !== 'managed') setManagedSessionMissing(false);
    setStatus({ kind: 'saving' });
    try {
      const result = await updateEmbeddingsSettings({
        provider: slug,
        model: newModel,
        dimensions: newDims,
        confirm_wipe: false,
      });
      if (result.error === 'EMBEDDINGS_DIMENSION_CHANGE_REQUIRES_WIPE') {
        setPendingWipe({ provider: slug, model: newModel, dimensions: newDims });
        setStatus({ kind: 'idle' });
        return;
      }
      await reload();
      setStatus({ kind: 'saved' });
    } catch (err) {
      setStatus({ kind: 'error', message: err instanceof Error ? err.message : String(err) });
    }
  }

  async function handleModelChange(modelId: string) {
    const model = currentModels.find(m => m.id === modelId);
    const newDims = model?.default_dimensions ?? settings!.dimensions;
    setStatus({ kind: 'saving' });
    try {
      const result = await updateEmbeddingsSettings({
        model: modelId,
        dimensions: newDims,
        confirm_wipe: false,
      });
      if (result.error === 'EMBEDDINGS_DIMENSION_CHANGE_REQUIRES_WIPE') {
        setPendingWipe({ model: modelId, dimensions: newDims });
        setStatus({ kind: 'idle' });
        return;
      }
      await reload();
      setStatus({ kind: 'saved' });
    } catch (err) {
      setStatus({ kind: 'error', message: err instanceof Error ? err.message : String(err) });
    }
  }

  async function handleDimsChange(dims: number) {
    setStatus({ kind: 'saving' });
    try {
      const result = await updateEmbeddingsSettings({ dimensions: dims, confirm_wipe: false });
      if (result.error === 'EMBEDDINGS_DIMENSION_CHANGE_REQUIRES_WIPE') {
        setPendingWipe({ dimensions: dims });
        setStatus({ kind: 'idle' });
        return;
      }
      await reload();
      setStatus({ kind: 'saved' });
    } catch (err) {
      setStatus({ kind: 'error', message: err instanceof Error ? err.message : String(err) });
    }
  }

  async function confirmWipe() {
    if (!pendingWipe) return;
    setStatus({ kind: 'saving' });
    const wipe = pendingWipe;
    setPendingWipe(null);
    try {
      await updateEmbeddingsSettings({ ...wipe, confirm_wipe: true });
      await reload();
      setStatus({ kind: 'saved' });
    } catch (err) {
      setStatus({ kind: 'error', message: err instanceof Error ? err.message : String(err) });
    }
  }

  // ── Setup popup handlers ──

  async function setupTest() {
    if (!setupProvider) return;
    setSetupTesting(true);
    setSetupTestResult(null);
    setSetupError('');
    try {
      // Store the key first so the backend can use it for the test
      if (setupKey.trim()) {
        await setEmbeddingsApiKey(setupProvider.slug, setupKey.trim());
      }
      const defaultModel = setupProvider.models[0];
      const result = await testEmbeddingsConnection({
        provider: setupProvider.slug,
        model: defaultModel?.id,
        dimensions: defaultModel?.default_dimensions,
      });
      setSetupTestResult(result);
      if (result.success) {
        // Refresh settings to pick up the stored key
        await reload();
      }
    } catch (err) {
      setSetupError(err instanceof Error ? err.message : String(err));
    } finally {
      setSetupTesting(false);
    }
  }

  async function setupSave() {
    if (!setupProvider) return;
    setSetupSaving(true);
    setSetupError('');
    try {
      // Store key if not already stored during test
      if (setupKey.trim()) {
        await setEmbeddingsApiKey(setupProvider.slug, setupKey.trim());
      }
      // Switch to this provider
      await doProviderSwitch(setupProvider.slug);
      setSetupProvider(null);
      setSetupKey('');
      setSetupTestResult(null);
    } catch (err) {
      setSetupError(err instanceof Error ? err.message : String(err));
    } finally {
      setSetupSaving(false);
    }
  }

  async function setupSaveCustom() {
    if (!customEndpoint.trim()) return;
    setSetupSaving(true);
    setSetupError('');
    try {
      if (setupKey.trim()) {
        await setEmbeddingsApiKey('custom', setupKey.trim());
      }
      setStatus({ kind: 'saving' });
      log('setupSaveCustom: calling update_embeddings_settings (provider=custom)');
      const result = await updateEmbeddingsSettings({
        provider: 'custom',
        model: customModel || 'embedding',
        dimensions: Number(customDims) || 1024,
        custom_endpoint: customEndpoint.trim(),
        confirm_wipe: false,
      });
      // Safe to log the error *code* (an enum-like sentinel, e.g.
      // EMBEDDINGS_AUTH_FAILED) — it carries no endpoint/key/detail content.
      log(
        'setupSaveCustom: rpc returned error_code=%s',
        typeof result.error === 'string' && result.error !== '' ? result.error : 'none'
      );
      // Setup-time verification failed: the endpoint couldn't prove it can
      // embed, so the config was NOT saved. update_settings only ever returns an
      // `error` for a verification failure or the dimension-wipe confirm, so any
      // error code other than the wipe-confirm is a failed probe. Matching on the
      // shape (rather than an explicit allow-list) means the differentiated #5017
      // codes — EMBEDDINGS_MODEL_INCOMPATIBLE / _AUTH_FAILED / _ENDPOINT_UNREACHABLE
      // / _DIMENSION_MISMATCH, plus _ENDPOINT_NO_API and _NO_MODEL_LOADED — all
      // surface their actionable backend message instead of a new code being
      // silently treated as a save. Keep the setup popup open so the user can fix
      // it (pick an embeddings model, correct the key/endpoint, …) and retry.
      if (
        typeof result.error === 'string' &&
        result.error !== '' &&
        result.error !== 'EMBEDDINGS_DIMENSION_CHANGE_REQUIRES_WIPE'
      ) {
        // `result.message`/`result.detail` are backend-emitted (already
        // context-specific); only the generic fallback is frontend-owned UI
        // text, so route just that through useT() (#4056 CodeRabbit).
        const baseMessage =
          typeof result.message === 'string'
            ? result.message
            : t('settings.embeddings.verifyFallback');
        // Append the underlying probe failure (HTTP status / server error body)
        // so the user can self-diagnose instead of seeing only the generic
        // message (#4056).
        setSetupError(
          typeof result.detail === 'string' && result.detail.trim()
            ? `${baseMessage} (${result.detail})`
            : baseMessage
        );
        // Verification failed: keep the setup popup open (status→idle, not error)
        // so the user can correct the model/key/endpoint and retry.
        log(
          'setupSaveCustom: verification failed (code=%s) — preserving setup popup, early return',
          result.error
        );
        setStatus({ kind: 'idle' });
        return;
      }
      if (result.error === 'EMBEDDINGS_DIMENSION_CHANGE_REQUIRES_WIPE') {
        log('setupSaveCustom: dimension change requires wipe — prompting confirm');
        setPendingWipe({
          provider: 'custom',
          model: customModel || 'embedding',
          dimensions: Number(customDims) || 1024,
          custom_endpoint: customEndpoint.trim(),
        });
        setStatus({ kind: 'idle' });
      } else {
        log('setupSaveCustom: verification passed — saved, reloading settings');
        await reload();
        setStatus({ kind: 'saved' });
      }
      setSetupProvider(null);
    } catch (err) {
      setSetupError(err instanceof Error ? err.message : String(err));
    } finally {
      setSetupSaving(false);
    }
  }

  async function handleClearKey() {
    if (!currentEntry) return;
    setStatus({ kind: 'saving' });
    try {
      await clearEmbeddingsApiKey(selectedProvider);
      await reload();
      setStatus({ kind: 'saved' });
    } catch (err) {
      setStatus({ kind: 'error', message: err instanceof Error ? err.message : String(err) });
    }
  }

  async function handleTestConnection() {
    setStatus({ kind: 'saving' });
    try {
      const result = await testEmbeddingsConnection();
      if (result.success) {
        setManagedSessionMissing(false);
        setStatus({ kind: 'saved' });
      } else {
        const message = result.error ?? t('settings.embeddings.connectionTestFailed');
        if (selectedProvider === 'managed' && isBackendSessionError(message)) {
          setManagedSessionMissing(true);
          setStatus({ kind: 'error', message: managedLoginMessage });
        } else {
          setStatus({ kind: 'error', message });
        }
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      if (selectedProvider === 'managed' && isBackendSessionError(message)) {
        setManagedSessionMissing(true);
        setStatus({ kind: 'error', message: managedLoginMessage });
      } else {
        setStatus({ kind: 'error', message });
      }
    }
  }

  return (
    <PanelPage
      className="z-10"
      contentClassName=""
      description={embedded ? undefined : t('pages.settings.ai.embeddingsDesc')}
      leading={embedded ? undefined : <SettingsBackButton onBack={navigateBack} />}>
      <div className={embedded ? 'space-y-5' : 'p-4 space-y-5'}>
        <p className="text-xs text-content-muted leading-relaxed">
          {t('settings.embeddings.description')}
        </p>

        {/* Provider selection */}
        <EmbeddingsProviderList
          providers={settings.providers}
          selectedProvider={selectedProvider}
          isLocalSession={isLocalSession}
          onSelect={handleProviderClick}
        />

        {showManagedLoginPrompt && (
          <Alert
            variant="warning"
            className="flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
            <AlertDescription className="opacity-100">
              {t('settings.embeddings.managedBannerIntro')}{' '}
              {isLocalSession
                ? t('settings.embeddings.managedBannerLocalSession')
                : t('settings.embeddings.managedBannerRemoteSession')}
            </AlertDescription>
            <Button
              variant="secondary"
              size="xs"
              className="shrink-0"
              onClick={() => void clearSession()}>
              {isLocalSession
                ? t('settings.exitLocalSession')
                : t('settings.embeddings.signInAgain')}
            </Button>
          </Alert>
        )}

        {/* Vector search disabled notice */}
        {selectedProvider === 'none' && (
          <Alert variant="warning">{t('settings.embeddings.vectorSearchDisabled')}</Alert>
        )}

        {/* Model & dimensions (for active provider with catalog models) */}
        {currentModels.length > 0 &&
          selectedProvider !== 'custom' &&
          selectedProvider !== 'none' && (
            <EmbeddingsModelSection
              currentModels={currentModels}
              allowedDims={allowedDims}
              model={settings.model}
              dimensions={settings.dimensions}
              onModelChange={modelId => void handleModelChange(modelId)}
              onDimsChange={dims => void handleDimsChange(dims)}
              canClearKey={Boolean(currentEntry?.requires_api_key && currentEntry.has_api_key)}
              onClearKey={() => void handleClearKey()}
              onTestConnection={() => void handleTestConnection()}
              testConnectionDisabled={selectedProvider === 'none' || managedRequiresLogin}
            />
          )}

        {/* Status bar */}
        <SettingsStatusLine
          saving={status.kind === 'saving'}
          savedNote={status.kind === 'saved' ? t('settings.embeddings.saved') : null}
          error={
            status.kind === 'error'
              ? `${t('settings.embeddings.errorPrefix')}: ${status.message}`
              : null
          }
          savingLabel={t('settings.embeddings.saving')}
        />
      </div>

      {/* ── Setup popup (API key entry + test + save) ── */}
      {setupProvider && (
        <EmbeddingsSetupModal
          setupProvider={setupProvider}
          onClose={() => setSetupProvider(null)}
          setupKey={setupKey}
          onSetupKeyChange={setSetupKey}
          setupShowKey={setupShowKey}
          onToggleShowKey={() => setSetupShowKey(s => !s)}
          setupTesting={setupTesting}
          setupTestResult={setupTestResult}
          setupSaving={setupSaving}
          setupError={setupError}
          customEndpoint={customEndpoint}
          onCustomEndpointChange={setCustomEndpoint}
          customModel={customModel}
          onCustomModelChange={setCustomModel}
          customDims={customDims}
          onCustomDimsChange={setCustomDims}
          onTest={() => void setupTest()}
          onSave={() => {
            if (setupProvider.slug === 'custom') {
              void setupSaveCustom();
            } else {
              void setupSave();
            }
          }}
        />
      )}
      {/* ── Confirm wipe dialog ── */}
      {pendingWipe && (
        <ConfirmDialog
          titleId="embeddings-wipe-title"
          title={t('settings.embeddings.wipeTitle')}
          body={t('settings.embeddings.wipeBody')}
          confirmLabel={t('settings.embeddings.confirmWipe')}
          cancelLabel={t('settings.embeddings.cancel')}
          destructive
          onConfirm={() => void confirmWipe()}
          onCancel={() => setPendingWipe(null)}
        />
      )}
    </PanelPage>
  );
};

function normalizeProvider(raw: string): string {
  if (raw === 'cloud') return 'managed';
  if (raw.startsWith('custom:')) return 'custom';
  return raw;
}

export default EmbeddingsPanel;
