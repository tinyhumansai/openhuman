/**
 * Embeddings settings panel — provider selection, API keys, model + dimensions.
 *
 * Flow: select a provider → if it needs an API key, a setup popup appears
 * to enter the key, test connection, and save. Dimension changes show a
 * destructive confirm dialog since they invalidate stored vectors.
 */
import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
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
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

type Status =
  | { kind: 'idle' }
  | { kind: 'loading' }
  | { kind: 'saving' }
  | { kind: 'saved' }
  | { kind: 'error'; message: string };

interface EmbeddingsPanelProps {
  embedded?: boolean;
}

const EmbeddingsPanel = ({ embedded = false }: EmbeddingsPanelProps = {}) => {
  const { t } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();

  const [settings, setSettings] = useState<EmbeddingsSettings | null>(null);
  const [status, setStatus] = useState<Status>({ kind: 'loading' });

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
      <div className="z-10 relative">
        {!embedded && (
          <SettingsHeader
            title={t('settings.embeddings.title')}
            showBackButton
            onBack={navigateBack}
            breadcrumbs={breadcrumbs}
          />
        )}
        <div className={embedded ? '' : 'p-4'}>
          <div className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-4 text-xs text-stone-500 dark:text-neutral-400">
            {status.kind === 'loading'
              ? t('common.loading')
              : status.kind === 'error'
                ? status.message
                : ''}
          </div>
        </div>
      </div>
    );
  }

  const selectedProvider = normalizeProvider(settings.provider);
  const currentEntry = settings.providers.find(p => p.slug === selectedProvider);
  const currentModels = currentEntry?.models ?? [];
  const currentModel = currentModels.find(m => m.id === settings.model) ?? currentModels[0];
  const allowedDims = currentModel?.allowed_dimensions ?? [];

  function handleProviderClick(entry: EmbeddingProviderEntry) {
    if (entry.slug === selectedProvider) return;

    if (entry.slug === 'custom') {
      // For custom, open setup popup to enter endpoint
      setSetupProvider(entry);
      setSetupKey('');
      setSetupTestResult(null);
      setSetupError('');
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
      const result = await updateEmbeddingsSettings({
        provider: 'custom',
        model: customModel || 'embedding',
        dimensions: Number(customDims) || 1024,
        custom_endpoint: customEndpoint.trim(),
        confirm_wipe: false,
      });
      if (result.error === 'EMBEDDINGS_DIMENSION_CHANGE_REQUIRES_WIPE') {
        setPendingWipe({
          provider: 'custom',
          model: customModel || 'embedding',
          dimensions: Number(customDims) || 1024,
          custom_endpoint: customEndpoint.trim(),
        });
        setStatus({ kind: 'idle' });
      } else {
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
        setStatus({ kind: 'saved' });
      } else {
        setStatus({ kind: 'error', message: result.error ?? 'Test failed' });
      }
    } catch (err) {
      setStatus({ kind: 'error', message: err instanceof Error ? err.message : String(err) });
    }
  }

  return (
    <div className="z-10 relative">
      {!embedded && (
        <SettingsHeader
          title={t('settings.embeddings.title')}
          showBackButton
          onBack={navigateBack}
          breadcrumbs={breadcrumbs}
        />
      )}

      <div className={embedded ? 'space-y-4' : 'p-4 space-y-4'}>
        <p className="text-xs text-stone-500 dark:text-neutral-400 leading-relaxed">
          {t('settings.embeddings.description')}
        </p>

        {/* Provider selection */}
        <div
          className="bg-white dark:bg-neutral-900 rounded-xl border border-neutral-200 dark:border-neutral-800 overflow-hidden"
          role="radiogroup"
          aria-label={t('settings.embeddings.providerAria')}>
          {settings.providers.map((entry, idx) => {
            const selected = entry.slug === selectedProvider;
            return (
              <button
                key={entry.slug}
                type="button"
                role="radio"
                aria-checked={selected}
                onClick={() => handleProviderClick(entry)}
                className={`w-full flex items-start gap-3 px-4 py-3 text-left transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-primary-500 ${
                  idx !== 0 ? 'border-t border-neutral-100 dark:border-neutral-800' : ''
                } ${
                  selected
                    ? 'bg-primary-50 dark:bg-primary-500/10'
                    : 'hover:bg-neutral-50 dark:hover:bg-neutral-800/60'
                }`}>
                <span className="flex-1 min-w-0">
                  <span className="flex items-center gap-2">
                    <span className="text-sm font-medium text-neutral-900 dark:text-neutral-100">
                      {entry.label}
                    </span>
                    {entry.requires_api_key && (
                      <span
                        className={`inline-flex items-center px-1.5 py-0.5 rounded text-[9px] font-semibold uppercase tracking-wider ${
                          entry.has_api_key
                            ? 'bg-sage-100 text-sage-700 dark:bg-sage-900/40 dark:text-sage-200'
                            : 'bg-amber-100 text-amber-800 dark:bg-amber-900/40 dark:text-amber-200'
                        }`}>
                        {entry.has_api_key
                          ? t('settings.embeddings.statusConfigured')
                          : t('settings.embeddings.statusNeedsKey')}
                      </span>
                    )}
                  </span>
                  <span className="block mt-0.5 text-xs text-neutral-500 dark:text-neutral-400">
                    {entry.description}
                  </span>
                </span>
                {selected && (
                  <svg
                    className="w-5 h-5 text-primary-500 flex-shrink-0 mt-0.5"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24"
                    aria-hidden>
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M5 13l4 4L19 7"
                    />
                  </svg>
                )}
              </button>
            );
          })}
        </div>

        {/* Vector search disabled notice */}
        {selectedProvider === 'none' && (
          <div className="rounded-xl border border-amber-200 dark:border-amber-500/30 bg-amber-50 dark:bg-amber-900/10 p-3">
            <p className="text-xs text-amber-800 dark:text-amber-200 leading-relaxed">
              {t('settings.embeddings.vectorSearchDisabled')}
            </p>
          </div>
        )}

        {/* Model & dimensions (for active provider with catalog models) */}
        {currentModels.length > 0 &&
          selectedProvider !== 'custom' &&
          selectedProvider !== 'none' && (
            <div className="rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-3 space-y-3">
              {currentModels.length > 1 && (
                <div>
                  <label className="block text-xs font-semibold text-stone-700 dark:text-neutral-200 mb-1">
                    {t('settings.embeddings.model')}
                  </label>
                  <select
                    value={settings.model}
                    onChange={e => void handleModelChange(e.target.value)}
                    className="w-full px-2 py-1.5 rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 text-xs text-stone-900 dark:text-neutral-100 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500">
                    {currentModels.map(m => (
                      <option key={m.id} value={m.id}>
                        {m.label} ({m.id})
                      </option>
                    ))}
                  </select>
                </div>
              )}

              {allowedDims.length > 1 && (
                <div>
                  <label className="block text-xs font-semibold text-stone-700 dark:text-neutral-200 mb-1">
                    {t('settings.embeddings.dimensions')}
                  </label>
                  <select
                    value={settings.dimensions}
                    onChange={e => void handleDimsChange(Number(e.target.value))}
                    className="w-full px-2 py-1.5 rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 text-xs text-stone-900 dark:text-neutral-100 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500">
                    {allowedDims.map(d => (
                      <option key={d} value={d}>
                        {d}
                      </option>
                    ))}
                  </select>
                </div>
              )}

              {/* Active provider info + actions */}
              <div className="flex items-center gap-2 pt-1">
                {currentEntry?.requires_api_key && currentEntry.has_api_key && (
                  <button
                    type="button"
                    onClick={() => void handleClearKey()}
                    className="px-2.5 py-1 rounded-md border border-coral-200 dark:border-coral-500/30 text-[11px] text-coral-600 dark:text-coral-300 hover:bg-coral-50 dark:hover:bg-coral-500/10">
                    {t('settings.embeddings.clearKey')}
                  </button>
                )}
                <button
                  type="button"
                  onClick={() => void handleTestConnection()}
                  disabled={selectedProvider === 'none'}
                  className="px-2.5 py-1 rounded-md border border-stone-200 dark:border-neutral-800 text-[11px] text-stone-700 dark:text-neutral-200 hover:bg-stone-50 dark:hover:bg-neutral-800 disabled:opacity-50">
                  {t('settings.embeddings.testConnection')}
                </button>
              </div>
            </div>
          )}

        {/* Status bar */}
        <div
          role="status"
          aria-live="polite"
          className="text-xs min-h-[1rem] text-stone-500 dark:text-neutral-400">
          {status.kind === 'saving' && t('settings.embeddings.saving')}
          {status.kind === 'saved' && t('settings.embeddings.saved')}
          {status.kind === 'error' && (
            <span className="text-coral-600 dark:text-coral-300">
              {t('settings.embeddings.errorPrefix')}: {status.message}
            </span>
          )}
        </div>
      </div>

      {/* ── Setup popup (API key entry + test + save) ── */}
      {setupProvider && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
          onClick={e => {
            if (e.target === e.currentTarget) {
              setSetupProvider(null);
            }
          }}>
          <div className="mx-4 max-w-md w-full rounded-2xl bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-700 p-6 shadow-xl space-y-4">
            <h3 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">
              {t('settings.embeddings.setupTitle').replace('{provider}', setupProvider.label)}
            </h3>

            {setupProvider.slug === 'custom' ? (
              /* Custom endpoint form */
              <div className="space-y-3">
                <div>
                  <label className="block text-[11px] font-medium text-stone-600 dark:text-neutral-300 mb-1">
                    {t('settings.embeddings.customEndpoint')}
                  </label>
                  <input
                    type="text"
                    value={customEndpoint}
                    onChange={e => setCustomEndpoint(e.target.value)}
                    placeholder="https://your-endpoint.com/v1"
                    className="w-full px-2.5 py-1.5 rounded-md border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-800 text-xs font-mono text-stone-900 dark:text-neutral-100 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
                    autoFocus
                  />
                </div>
                <div className="flex gap-2">
                  <div className="flex-1">
                    <label className="block text-[11px] font-medium text-stone-600 dark:text-neutral-300 mb-1">
                      {t('settings.embeddings.customModelPlaceholder')}
                    </label>
                    <input
                      type="text"
                      value={customModel}
                      onChange={e => setCustomModel(e.target.value)}
                      placeholder="text-embedding-3-small"
                      className="w-full px-2.5 py-1.5 rounded-md border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-800 text-xs font-mono text-stone-900 dark:text-neutral-100 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
                    />
                  </div>
                  <div className="w-24">
                    <label className="block text-[11px] font-medium text-stone-600 dark:text-neutral-300 mb-1">
                      {t('settings.embeddings.dimensions')}
                    </label>
                    <input
                      type="number"
                      value={customDims}
                      onChange={e => setCustomDims(e.target.value)}
                      placeholder="1024"
                      className="w-full px-2.5 py-1.5 rounded-md border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-800 text-xs font-mono text-stone-900 dark:text-neutral-100 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
                    />
                  </div>
                </div>
                <div>
                  <label className="block text-[11px] font-medium text-stone-600 dark:text-neutral-300 mb-1">
                    {t('settings.embeddings.apiKeyLabel').replace('{provider}', 'API')} (
                    {t('settings.embeddings.optional')})
                  </label>
                  <input
                    type={setupShowKey ? 'text' : 'password'}
                    value={setupKey}
                    onChange={e => setSetupKey(e.target.value)}
                    placeholder={t('settings.embeddings.placeholderKey')}
                    className="w-full px-2.5 py-1.5 rounded-md border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-800 text-xs font-mono text-stone-900 dark:text-neutral-100 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
                  />
                </div>
              </div>
            ) : (
              /* Standard API key form */
              <div className="space-y-3">
                <p className="text-xs text-neutral-500 dark:text-neutral-400">
                  {setupProvider.description}
                </p>
                <div>
                  <label className="block text-[11px] font-medium text-stone-600 dark:text-neutral-300 mb-1">
                    {t('settings.embeddings.apiKeyLabel').replace(
                      '{provider}',
                      setupProvider.label
                    )}
                  </label>
                  <div className="flex gap-2">
                    <input
                      type={setupShowKey ? 'text' : 'password'}
                      value={setupKey}
                      onChange={e => setSetupKey(e.target.value)}
                      placeholder={t('settings.embeddings.placeholderKey')}
                      className="flex-1 px-2.5 py-1.5 rounded-md border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-800 text-xs font-mono text-stone-900 dark:text-neutral-100 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
                      autoFocus
                    />
                    <button
                      type="button"
                      onClick={() => setSetupShowKey(s => !s)}
                      className="px-2 py-1.5 rounded-md border border-stone-200 dark:border-neutral-700 text-xs text-stone-600 dark:text-neutral-300 hover:bg-stone-50 dark:hover:bg-neutral-800">
                      {setupShowKey ? t('settings.embeddings.hide') : t('settings.embeddings.show')}
                    </button>
                  </div>
                  <p className="mt-1 text-[10px] text-stone-400 dark:text-neutral-500">
                    {t('settings.embeddings.keyStoredEncrypted')}
                  </p>
                </div>
              </div>
            )}

            {/* Test result */}
            {setupTestResult && (
              <div
                className={`rounded-lg px-3 py-2 text-xs ${
                  setupTestResult.success
                    ? 'bg-sage-50 dark:bg-sage-900/20 text-sage-700 dark:text-sage-300'
                    : 'bg-coral-50 dark:bg-coral-900/20 text-coral-700 dark:text-coral-300'
                }`}>
                {setupTestResult.success
                  ? t('settings.embeddings.testSuccess').replace(
                      '{dims}',
                      String(setupTestResult.actual_dimensions ?? '?')
                    )
                  : t('settings.embeddings.testFailed').replace(
                      '{error}',
                      setupTestResult.error ?? ''
                    )}
              </div>
            )}

            {setupError && (
              <div className="rounded-lg px-3 py-2 text-xs bg-coral-50 dark:bg-coral-900/20 text-coral-700 dark:text-coral-300">
                {setupError}
              </div>
            )}

            {/* Popup actions */}
            <div className="flex justify-between pt-1">
              <button
                type="button"
                onClick={() => {
                  if (setupProvider.slug !== 'custom') {
                    void setupTest();
                  }
                }}
                disabled={
                  setupTesting ||
                  setupSaving ||
                  (setupProvider.slug !== 'custom' && !setupKey.trim())
                }
                className="px-3 py-1.5 rounded-lg text-xs font-medium border border-stone-200 dark:border-neutral-700 text-stone-700 dark:text-neutral-200 hover:bg-stone-50 dark:hover:bg-neutral-800 disabled:opacity-40">
                {setupTesting
                  ? t('settings.embeddings.testing')
                  : t('settings.embeddings.testConnection')}
              </button>

              <div className="flex gap-2">
                <button
                  type="button"
                  onClick={() => setSetupProvider(null)}
                  className="px-4 py-1.5 rounded-lg text-xs font-medium text-neutral-600 dark:text-neutral-300 hover:bg-neutral-100 dark:hover:bg-neutral-800">
                  {t('settings.embeddings.cancel')}
                </button>
                <button
                  type="button"
                  onClick={() => {
                    if (setupProvider.slug === 'custom') {
                      void setupSaveCustom();
                    } else {
                      void setupSave();
                    }
                  }}
                  disabled={
                    setupSaving ||
                    (setupProvider.slug !== 'custom' &&
                      !setupKey.trim() &&
                      !setupProvider.has_api_key) ||
                    (setupProvider.slug === 'custom' && !customEndpoint.trim())
                  }
                  className="px-4 py-1.5 rounded-lg text-xs font-medium bg-primary-500 hover:bg-primary-600 text-white disabled:opacity-40">
                  {setupSaving
                    ? t('settings.embeddings.saving')
                    : t('settings.embeddings.saveAndSwitch')}
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* ── Confirm wipe dialog ── */}
      {pendingWipe && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
          <div className="mx-4 max-w-sm w-full rounded-2xl bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-700 p-6 shadow-xl space-y-4">
            <h3 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">
              {t('settings.embeddings.wipeTitle')}
            </h3>
            <p className="text-xs text-neutral-600 dark:text-neutral-400 leading-relaxed">
              {t('settings.embeddings.wipeBody')}
            </p>
            <div className="flex justify-end gap-2">
              <button
                type="button"
                onClick={() => setPendingWipe(null)}
                className="px-4 py-2 rounded-lg text-xs font-medium text-neutral-600 dark:text-neutral-300 hover:bg-neutral-100 dark:hover:bg-neutral-800">
                {t('settings.embeddings.cancel')}
              </button>
              <button
                type="button"
                onClick={() => void confirmWipe()}
                className="px-4 py-2 rounded-lg text-xs font-medium bg-coral-500 hover:bg-coral-600 text-white">
                {t('settings.embeddings.confirmWipe')}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};

function normalizeProvider(raw: string): string {
  if (raw === 'cloud') return 'managed';
  if (raw.startsWith('custom:')) return 'custom';
  return raw;
}

export default EmbeddingsPanel;
