/**
 * Setup popup for the Embeddings panel — API key entry (+ test + save) for a
 * catalog provider, or the custom-endpoint form. Extracted out of
 * `EmbeddingsPanel.tsx` to keep that file under the repo's file-size
 * convention; all state stays owned by the parent panel and is threaded
 * through as props.
 */
import { useT } from '../../../lib/i18n/I18nContext';
import type {
  EmbeddingProviderEntry,
  EmbeddingsTestResult,
} from '../../../services/api/embeddingsApi';
import { Alert, Button, Label, ModalShell } from '../../ui';
import { SettingsTextField } from '../controls';

export interface EmbeddingsSetupModalProps {
  setupProvider: EmbeddingProviderEntry;
  onClose: () => void;

  setupKey: string;
  onSetupKeyChange: (next: string) => void;
  setupShowKey: boolean;
  onToggleShowKey: () => void;
  setupTesting: boolean;
  setupTestResult: EmbeddingsTestResult | null;
  setupSaving: boolean;
  setupError: string;

  customEndpoint: string;
  onCustomEndpointChange: (next: string) => void;
  customModel: string;
  onCustomModelChange: (next: string) => void;
  customDims: string;
  onCustomDimsChange: (next: string) => void;

  onTest: () => void;
  onSave: () => void;
}

const EmbeddingsSetupModal = ({
  setupProvider,
  onClose,
  setupKey,
  onSetupKeyChange,
  setupShowKey,
  onToggleShowKey,
  setupTesting,
  setupTestResult,
  setupSaving,
  setupError,
  customEndpoint,
  onCustomEndpointChange,
  customModel,
  onCustomModelChange,
  customDims,
  onCustomDimsChange,
  onTest,
  onSave,
}: EmbeddingsSetupModalProps) => {
  const { t } = useT();
  const isCustom = setupProvider.slug === 'custom';

  return (
    <ModalShell
      titleId="embeddings-setup-title"
      title={t('settings.embeddings.setupTitle').replace('{provider}', setupProvider.label)}
      onClose={onClose}
      contentClassName="px-5 py-4 space-y-4"
      footer={
        <div className="flex justify-between pt-1">
          {/* Disabled for a custom endpoint, deliberately. `setupTest` calls
              `openhuman.embeddings_test_connection` with only
              `{ provider, model, dimensions }` — there is no parameter for an
              endpoint URL — so a custom provider has nothing to test against.
              Previously the button stayed enabled and its handler opened with
              `if (!isCustom)`, so a click did nothing at all: no request, no
              result, no error.

              The reason is rendered as visible text beside the button, not as a
              `title` on it. `Button` applies `disabled:pointer-events-none`
              (ui/Button.tsx:40), so a disabled control cannot be hovered — and a
              disabled button is out of the tab order, so a `title` is
              unreachable by keyboard too. Text that is simply on screen needs
              neither. */}
          <div className="flex items-center gap-2">
            <Button
              variant="secondary"
              size="xs"
              onClick={onTest}
              disabled={setupTesting || setupSaving || isCustom || !setupKey.trim()}>
              {setupTesting
                ? t('settings.embeddings.testing')
                : t('settings.embeddings.testConnection')}
            </Button>
            {isCustom && (
              <span
                data-testid="embeddings-test-unavailable-reason"
                className="max-w-[18rem] text-[11px] text-content-muted">
                {t(
                  'settings.embeddings.testUnavailableCustom',
                  'Testing a custom endpoint is not supported yet — save it and check the status on the Embeddings panel.'
                )}
              </span>
            )}
          </div>

          <div className="flex gap-2">
            <Button variant="tertiary" size="xs" onClick={onClose}>
              {t('settings.embeddings.cancel')}
            </Button>
            <Button
              variant="primary"
              size="xs"
              onClick={onSave}
              disabled={
                setupSaving ||
                (!isCustom && !setupKey.trim() && !setupProvider.has_api_key) ||
                (isCustom && !customEndpoint.trim())
              }>
              {setupSaving
                ? t('settings.embeddings.saving')
                : t('settings.embeddings.saveAndSwitch')}
            </Button>
          </div>
        </div>
      }>
      {isCustom ? (
        /* Custom endpoint form */
        <div className="space-y-3">
          <div>
            <Label className="block text-[11px] mb-1">
              {t('settings.embeddings.customEndpoint')}
            </Label>
            <SettingsTextField
              type="text"
              value={customEndpoint}
              onChange={e => onCustomEndpointChange(e.target.value)}
              placeholder="https://your-endpoint.com/v1"
              mono
              autoFocus
            />
          </div>
          <div className="flex gap-2">
            <div className="flex-1">
              <Label className="block text-[11px] mb-1">
                {t('settings.embeddings.customModelPlaceholder')}
              </Label>
              <SettingsTextField
                type="text"
                value={customModel}
                onChange={e => onCustomModelChange(e.target.value)}
                placeholder="text-embedding-3-small"
                mono
              />
            </div>
            <div className="w-24">
              <Label className="block text-[11px] mb-1">
                {t('settings.embeddings.dimensions')}
              </Label>
              <SettingsTextField
                type="number"
                value={customDims}
                onChange={e => onCustomDimsChange(e.target.value)}
                placeholder="1024"
                mono
              />
            </div>
          </div>
          <div>
            <Label className="block text-[11px] mb-1">
              {t('settings.embeddings.apiKeyLabelGeneric')} ({t('settings.embeddings.optional')})
            </Label>
            <SettingsTextField
              type={setupShowKey ? 'text' : 'password'}
              value={setupKey}
              onChange={e => onSetupKeyChange(e.target.value)}
              placeholder={t('settings.embeddings.placeholderKey')}
              mono
            />
          </div>
        </div>
      ) : (
        /* Standard API key form */
        <div className="space-y-3">
          <p className="text-xs text-content-muted">{setupProvider.description}</p>
          <div>
            <Label className="block text-[11px] mb-1">
              {t('settings.embeddings.apiKeyLabel').replace('{provider}', setupProvider.label)}
            </Label>
            <div className="flex gap-2">
              <SettingsTextField
                type={setupShowKey ? 'text' : 'password'}
                value={setupKey}
                onChange={e => onSetupKeyChange(e.target.value)}
                placeholder={t('settings.embeddings.placeholderKey')}
                mono
                autoFocus
                className="flex-1"
              />
              <Button variant="secondary" size="xs" onClick={onToggleShowKey}>
                {setupShowKey ? t('settings.embeddings.hide') : t('settings.embeddings.show')}
              </Button>
            </div>
            <p className="mt-1 text-[10px] text-content-faint">
              {t('settings.embeddings.keyStoredEncrypted')}
            </p>
          </div>
        </div>
      )}

      {/* Test result */}
      {setupTestResult && (
        <Alert variant={setupTestResult.success ? 'success' : 'destructive'} className="text-xs">
          {setupTestResult.success
            ? t('settings.embeddings.testSuccess').replace(
                '{dims}',
                String(setupTestResult.actual_dimensions ?? '?')
              )
            : t('settings.embeddings.testFailed').replace('{error}', setupTestResult.error ?? '')}
        </Alert>
      )}

      {setupError && (
        <Alert variant="destructive" className="text-xs">
          {setupError}
        </Alert>
      )}
    </ModalShell>
  );
};

export default EmbeddingsSetupModal;
