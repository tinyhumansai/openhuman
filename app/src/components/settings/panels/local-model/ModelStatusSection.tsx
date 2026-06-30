import { useT } from '../../../../lib/i18n/I18nContext';
import { formatBytes, statusLabel } from '../../../../utils/localAiHelpers';
import { validateOllamaUrl } from '../../../../utils/ollamaUrlValidation';
import type {
  LocalAiDiagnostics,
  LocalAiDownloadsProgress,
  LocalAiStatus,
  ModelContextEligibility,
  OllamaConnectionTestResult,
  RepairAction,
} from '../../../../utils/tauriCommands';
import Button from '../../../ui/Button';
import { SettingsSection, SettingsStatusLine, SettingsTextField } from '../../controls';

/**
 * Badge rendering a model's context-window verdict against the memory
 * layer minimum. `below_minimum` is a hard rejection (the memory pipeline
 * would silently truncate and corrupt recall); `unknown` is a soft warning.
 */
const ContextEligibilityBadge = ({
  eligibility,
}: {
  eligibility: ModelContextEligibility | null | undefined;
}) => {
  const { t } = useT();
  if (!eligibility) return null;
  const fmt = (n: number) => n.toLocaleString();
  if (eligibility.status === 'ok') {
    return (
      <span
        className="shrink-0 rounded-full bg-green-100 dark:bg-green-500/15 px-2 py-0.5 text-[10px] font-medium text-green-700 dark:text-green-300"
        title={t('settings.localModel.status.contextOkTitle')
          .replace('{contextLength}', fmt(eligibility.context_length))
          .replace('{required}', fmt(eligibility.context_length))}>
        {t('settings.localModel.status.contextOkBadge').replace(
          '{contextLength}',
          fmt(eligibility.context_length)
        )}
      </span>
    );
  }
  if (eligibility.status === 'below_minimum') {
    return (
      <span
        className="shrink-0 rounded-full bg-red-100 dark:bg-red-500/15 px-2 py-0.5 text-[10px] font-medium text-red-700 dark:text-red-300"
        title={t('settings.localModel.status.contextBelowMinimumTitle')
          .replace('{contextLength}', fmt(eligibility.context_length))
          .replace('{required}', fmt(eligibility.required))}>
        {t('settings.localModel.status.contextBelowMinimumBadge')
          .replace('{contextLength}', fmt(eligibility.context_length))
          .replace('{required}', fmt(eligibility.required))}
      </span>
    );
  }
  return (
    <span
      className="shrink-0 rounded-full bg-surface-strong dark:bg-neutral-700 px-2 py-0.5 text-[10px] font-medium text-content-secondary"
      title={t('settings.localModel.status.contextUnknownTitle').replace(
        '{required}',
        fmt(eligibility.required)
      )}>
      {t('settings.localModel.status.contextUnknownBadge')}
    </span>
  );
};

interface ModelStatusSectionProps {
  status: LocalAiStatus | null;
  downloads: LocalAiDownloadsProgress | null;
  diagnostics: LocalAiDiagnostics | null;
  isDiagnosticsLoading: boolean;
  diagnosticsError: string;
  statusError: string;
  isTriggeringDownload: boolean;
  bootstrapMessage: string;
  progress: number;
  isIndeterminateDownload: boolean;
  isInstalling: boolean;
  isInstallError: boolean;
  showErrorDetail: boolean;
  ollamaPathInput: string;
  isSettingPath: boolean;
  downloadedText: string;
  speedText: string;
  etaText: string;
  statusTone: (state: string) => string;
  runtimeEnabled: boolean;
  ollamaBaseUrlInput: string;
  isTestingConnection: boolean;
  connectionTestResult: OllamaConnectionTestResult | null;
  isSavingUrl: boolean;
  onRefreshStatus: () => void;
  onTriggerDownload: (force: boolean) => void;
  onSetOllamaPath: () => void;
  onClearOllamaPath: () => void;
  onSetOllamaPathInput: (value: string) => void;
  onToggleErrorDetail: () => void;
  onRunDiagnostics: () => void;
  onRepairAction?: (action: RepairAction) => void;
  onSetOllamaBaseUrlInput: (value: string) => void;
  onTestConnection: () => void;
  onSaveOllamaBaseUrl: () => void;
  onResetOllamaBaseUrl: () => void;
  savedOllamaBaseUrl: string;
}

const ModelStatusSection = ({
  status,
  downloads,
  diagnostics,
  isDiagnosticsLoading,
  diagnosticsError,
  statusError,
  isTriggeringDownload,
  bootstrapMessage,
  progress,
  isIndeterminateDownload,
  isInstalling,
  isInstallError,
  showErrorDetail,
  ollamaPathInput,
  isSettingPath,
  downloadedText,
  speedText,
  etaText,
  statusTone,
  runtimeEnabled,
  ollamaBaseUrlInput,
  isTestingConnection,
  connectionTestResult,
  isSavingUrl,
  onRefreshStatus,
  onTriggerDownload,
  onSetOllamaPath,
  onClearOllamaPath,
  onSetOllamaPathInput,
  onToggleErrorDetail,
  onRunDiagnostics,
  onRepairAction,
  onSetOllamaBaseUrlInput,
  onTestConnection,
  onSaveOllamaBaseUrl,
  onResetOllamaBaseUrl,
  savedOllamaBaseUrl,
}: ModelStatusSectionProps) => {
  const { t } = useT();
  // OpenHuman no longer installs or launches Ollama itself. When the runtime
  // is unavailable, surface manual guidance instead of management controls.
  const showInstallOllamaCta = downloads?.ollama_available === false;

  void isTriggeringDownload;
  void bootstrapMessage;
  void isInstalling;
  void isInstallError;
  void showErrorDetail;
  void ollamaPathInput;
  void isSettingPath;
  void runtimeEnabled;
  void onTriggerDownload;
  void onSetOllamaPath;
  void onClearOllamaPath;
  void onSetOllamaPathInput;
  void onToggleErrorDetail;
  void onRepairAction;

  const urlValidation = validateOllamaUrl(ollamaBaseUrlInput);
  const urlChanged = ollamaBaseUrlInput !== savedOllamaBaseUrl;
  const canSave = urlValidation.valid && urlChanged && !isSavingUrl;
  const canTest = urlValidation.valid && !isTestingConnection;

  if (showInstallOllamaCta) {
    return (
      <section className="rounded-lg border border-amber-300 dark:border-amber-500/40 bg-amber-50 dark:bg-amber-500/10 p-4 space-y-3">
        <div className="flex items-start gap-3">
          <svg
            className="h-5 w-5 flex-shrink-0 text-amber-600 dark:text-amber-300 mt-0.5"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"
            />
          </svg>
          <div className="flex-1 space-y-1">
            <div className="text-sm font-semibold text-amber-900">
              {t('settings.localModel.status.ollamaNotInstalled')}
            </div>
            <div className="text-xs text-amber-800 dark:text-amber-200">
              {t('settings.localModel.status.ollamaNotInstalledDesc')}
            </div>
          </div>
        </div>
        <div className="flex items-center gap-2 pt-1">
          <a
            href="https://ollama.com"
            target="_blank"
            rel="noopener noreferrer"
            className="px-3 py-1.5 text-xs rounded-md border border-amber-300 dark:border-amber-500/40 hover:border-amber-400 text-amber-800 dark:text-amber-200">
            {t('settings.localModel.status.ollamaDocs')}
          </a>
        </div>

        {isInstallError && status?.error_detail && (
          <div className="space-y-1 pt-2 border-t border-amber-200 dark:border-amber-500/30">
            <Button
              variant="tertiary"
              tone="danger"
              size="xs"
              onClick={onToggleErrorDetail}
              className="underline">
              {showErrorDetail
                ? t('settings.localModel.status.hideErrorDetails')
                : t('settings.localModel.status.showInstallErrorDetails')}
            </Button>
            {showErrorDetail && (
              <pre className="max-h-40 overflow-auto rounded bg-red-50 dark:bg-red-500/10 border border-red-200 dark:border-red-500/30 p-2 text-[10px] text-red-700 dark:text-red-300 leading-tight whitespace-pre-wrap break-words">
                {status.error_detail}
              </pre>
            )}
          </div>
        )}
      </section>
    );
  }

  return (
    <>
      <SettingsSection title={t('localModel.ollamaServer.label')}>
        <div className="px-4 py-3 space-y-3">
          <div className="space-y-1.5">
            <SettingsTextField
              type="text"
              value={ollamaBaseUrlInput}
              onChange={e => onSetOllamaBaseUrlInput(e.target.value)}
              placeholder={t('localModel.ollamaServer.placeholder')}
              invalid={!!(ollamaBaseUrlInput && !urlValidation.valid)}
              aria-label={t('localModel.ollamaServer.label')}
            />
            {ollamaBaseUrlInput && !urlValidation.valid && (
              <p className="text-xs text-red-600 dark:text-red-300">
                {urlValidation.error ?? t('localModel.ollamaServer.validationError')}
              </p>
            )}
            <p className="text-xs text-content-muted">{t('localModel.ollamaServer.helperText')}</p>
          </div>

          {connectionTestResult !== null && (
            <div
              className={`flex items-center gap-2 text-xs ${connectionTestResult.reachable ? 'text-green-600 dark:text-green-300' : 'text-red-600 dark:text-red-300'}`}>
              <span>{connectionTestResult.reachable ? '✓' : '✗'}</span>
              <span>
                {connectionTestResult.reachable
                  ? `${t('localModel.ollamaServer.reachable')}${typeof connectionTestResult.models_count === 'number' ? ` (${connectionTestResult.models_count} ${t('localModel.ollamaServer.modelCount')})` : ''}`
                  : `${t('localModel.ollamaServer.unreachable')}${connectionTestResult.error ? `: ${connectionTestResult.error}` : ''}`}
              </span>
            </div>
          )}

          <div className="flex items-center gap-2">
            <Button
              type="button"
              variant="secondary"
              size="xs"
              onClick={onTestConnection}
              disabled={!canTest}
              leadingIcon={
                isTestingConnection ? (
                  <span className="h-3 w-3 rounded-full border-2 border-current border-t-transparent animate-spin" />
                ) : undefined
              }>
              {t('localModel.ollamaServer.testButton')}
            </Button>
            <Button
              type="button"
              variant="primary"
              size="xs"
              onClick={onSaveOllamaBaseUrl}
              disabled={!canSave}>
              {t('localModel.ollamaServer.saveButton')}
            </Button>
            <Button
              variant="tertiary"
              size="xs"
              onClick={onResetOllamaBaseUrl}
              className="underline">
              {t('localModel.ollamaServer.resetButton')}
            </Button>
          </div>
        </div>
      </SettingsSection>

      <SettingsSection title={t('settings.localModel.status.runtimeStatus')}>
        <div className="px-4 py-3 space-y-3">
          <div className="flex justify-end">
            <Button type="button" variant="tertiary" size="xs" onClick={onRefreshStatus}>
              {t('common.refresh')}
            </Button>
          </div>
          <div className="flex items-center justify-between text-sm">
            <span className="text-content-muted">{t('settings.ai.state')}</span>
            <span className={`font-medium ${statusTone(status?.state ?? 'idle')}`}>
              {status
                ? statusLabel(downloads?.state ?? status.state)
                : t('settings.localModel.status.unavailable')}
            </span>
          </div>

          <div className="h-2 rounded-full bg-surface-strong overflow-hidden">
            <div
              className={`h-full bg-gradient-to-r from-blue-500 to-cyan-400 transition-all duration-500 ${
                isIndeterminateDownload ? 'animate-pulse' : ''
              }`}
              style={{ width: `${Math.round((isIndeterminateDownload ? 1 : progress) * 100)}%` }}
            />
          </div>

          <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-content-muted">
            <span>
              {t('settings.localModel.status.progress')}{' '}
              {isInstalling
                ? t('settings.localModel.status.installingOllama')
                : isIndeterminateDownload
                  ? t('settings.localModel.status.downloadingUnknown')
                  : `${Math.round(progress * 100)}%`}
            </span>
            {downloadedText && <span className="text-content-secondary">{downloadedText}</span>}
            {speedText && (
              <span className="text-primary-600 dark:text-primary-300">{speedText}</span>
            )}
            {etaText && (
              <span className="text-primary-500">
                {t('settings.localModel.status.eta')} {etaText}
              </span>
            )}
          </div>

          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 text-sm">
            <div className="rounded-md border border-line p-2">
              <div className="text-content-muted text-xs uppercase tracking-wide">
                {t('settings.localModel.status.provider')}
              </div>
              <div className="text-content mt-1">
                {status?.provider ?? t('settings.localModel.status.notAvailable')}
              </div>
            </div>
            <div className="rounded-md border border-line p-2">
              <div className="text-content-muted text-xs uppercase tracking-wide">
                {t('settings.localModel.status.model')}
              </div>
              <div className="text-content mt-1">
                {status?.model_id ?? t('settings.localModel.status.notAvailable')}
              </div>
            </div>
          </div>

          <div className="grid grid-cols-1 sm:grid-cols-3 gap-3 text-sm">
            <div className="rounded-md border border-line p-2">
              <div className="text-content-muted text-xs uppercase tracking-wide">
                {t('settings.localModel.status.backend')}
              </div>
              <div className="text-content mt-1">{status?.active_backend ?? 'cpu'}</div>
            </div>
            <div className="rounded-md border border-line p-2">
              <div className="text-content-muted text-xs uppercase tracking-wide">
                {t('settings.localModel.status.lastLatency')}
              </div>
              <div className="text-content mt-1">
                {typeof status?.last_latency_ms === 'number'
                  ? `${status.last_latency_ms} ms`
                  : t('settings.localModel.status.notAvailable')}
              </div>
            </div>
            <div className="rounded-md border border-line p-2">
              <div className="text-content-muted text-xs uppercase tracking-wide">
                {t('settings.localModel.status.generationTps')}
              </div>
              <div className="text-content mt-1">
                {typeof status?.gen_toks_per_sec === 'number'
                  ? `${status.gen_toks_per_sec.toFixed(1)} tok/s`
                  : t('settings.localModel.status.notAvailable')}
              </div>
            </div>
          </div>

          {status?.model_path && (
            <div className="text-xs text-content-muted break-all">
              {t('settings.localModel.status.artifact')} {status.model_path}
            </div>
          )}

          {status?.backend_reason && (
            <div className="text-xs text-primary-600 dark:text-primary-300">
              {status.backend_reason}
            </div>
          )}
          {status?.warning && (
            <div className="text-xs text-amber-700 dark:text-amber-300">{status.warning}</div>
          )}
          {statusError && <SettingsStatusLine saving={false} error={statusError} savingLabel="" />}

          {status?.error_detail && (
            <div className="space-y-1">
              <Button
                variant="tertiary"
                tone="danger"
                size="xs"
                onClick={onToggleErrorDetail}
                className="underline">
                {showErrorDetail
                  ? t('settings.localModel.status.hideErrorDetails')
                  : t('settings.localModel.status.showErrorDetails')}
              </Button>
              {showErrorDetail && (
                <pre className="max-h-40 overflow-auto rounded bg-red-50 dark:bg-red-500/10 border border-red-200 dark:border-red-500/30 p-2 text-[10px] text-red-600 dark:text-red-300 leading-tight whitespace-pre-wrap break-words">
                  {status.error_detail}
                </pre>
              )}
              <p className="text-xs text-content-muted">
                {t('settings.localModel.status.installManuallyFrom')}{' '}
                <a
                  href="https://ollama.com"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-primary-500 hover:text-primary-600 dark:text-primary-300 underline">
                  ollama.com
                </a>{' '}
                {t('settings.localModel.status.thenRetry')}
              </p>
            </div>
          )}
        </div>
      </SettingsSection>

      <SettingsSection title={t('settings.localModel.status.ollamaDiagnostics')}>
        <div className="px-4 py-3 space-y-3">
          <div className="flex justify-end">
            <Button
              type="button"
              variant="primary"
              size="xs"
              onClick={onRunDiagnostics}
              disabled={isDiagnosticsLoading}>
              {isDiagnosticsLoading
                ? t('settings.localModel.status.checking')
                : t('settings.localModel.status.runDiagnostics')}
            </Button>
          </div>
          {!diagnostics && !diagnosticsError && (
            <p className="text-xs text-content-muted">
              {t('settings.localModel.status.diagnosticsHint')}
            </p>
          )}
          {isDiagnosticsLoading && (
            <div className="flex items-center gap-2 text-xs text-primary-600 dark:text-primary-300">
              <div className="h-3 w-3 rounded-full border-2 border-blue-400 border-t-transparent animate-spin" />
              {t('settings.localModel.status.checkingOllama')}
            </div>
          )}
          {diagnosticsError && (
            <SettingsStatusLine saving={false} error={diagnosticsError} savingLabel="" />
          )}
          {diagnostics && (
            <>
              <div className="flex items-center gap-2 text-sm">
                <span
                  className={`inline-block h-2.5 w-2.5 rounded-full ${diagnostics.ok ? 'bg-green-400' : 'bg-red-400'}`}
                />
                <span
                  className={
                    diagnostics.ok
                      ? 'text-green-600 dark:text-green-300'
                      : 'text-red-600 dark:text-red-300'
                  }>
                  {diagnostics.ok
                    ? t('settings.localModel.status.allChecksPassed')
                    : t('settings.localModel.status.issuesFound').replace(
                        '{count}',
                        String(diagnostics.issues.length)
                      )}
                </span>
              </div>

              <div className="grid grid-cols-2 gap-2 text-xs">
                <div className="rounded-md border border-line p-2">
                  <div className="text-content-muted uppercase tracking-wide text-[10px]">
                    {t('settings.localModel.status.server')}
                  </div>
                  <div
                    className={`mt-1 font-medium ${diagnostics.ollama_running ? 'text-green-600 dark:text-green-300' : 'text-red-600 dark:text-red-300'}`}>
                    {diagnostics.ollama_running
                      ? t('settings.localModel.status.running')
                      : t('settings.localModel.status.notRunning')}
                  </div>
                  {diagnostics.ollama_base_url && (
                    <div
                      className="mt-0.5 text-content-muted truncate text-[10px]"
                      title={diagnostics.ollama_base_url}>
                      {diagnostics.ollama_base_url}
                    </div>
                  )}
                </div>
                <div className="rounded-md border border-line p-2">
                  <div className="text-content-muted uppercase tracking-wide text-[10px]">
                    {t('settings.localModel.status.binary')}
                  </div>
                  <div
                    className="mt-1 text-content-secondary truncate"
                    title={
                      diagnostics.ollama_binary_path ??
                      (diagnostics.ollama_running
                        ? t('settings.localModel.status.externalProcess')
                        : t('settings.localModel.status.notFound'))
                    }>
                    {diagnostics.ollama_binary_path === null
                      ? diagnostics.ollama_running
                        ? t('settings.localModel.status.runningExternalProcess')
                        : t('settings.localModel.status.notFound')
                      : diagnostics.ollama_binary_path}
                  </div>
                </div>
              </div>

              {diagnostics.installed_models.length > 0 && (
                <div>
                  <div className="text-content-muted uppercase tracking-wide text-[10px] mb-1">
                    {t('settings.localModel.status.installedModels')} (
                    {diagnostics.installed_models.length})
                  </div>
                  <div className="space-y-1">
                    {diagnostics.installed_models.map(m => {
                      const rejected = m.eligibility?.status === 'below_minimum';
                      return (
                        <div
                          key={m.name}
                          className={`flex items-center justify-between gap-2 rounded border px-2 py-1.5 text-xs ${
                            rejected
                              ? 'border-red-300 dark:border-red-500/40 bg-red-50 dark:bg-red-500/10'
                              : 'border-line'
                          }`}>
                          <span
                            className={`min-w-0 truncate font-medium ${
                              rejected ? 'text-red-700 dark:text-red-300' : 'text-content'
                            }`}
                            title={m.name}>
                            {m.name}
                          </span>
                          <span className="flex shrink-0 items-center gap-2">
                            <ContextEligibilityBadge eligibility={m.eligibility} />
                            <span className="text-content-muted">
                              {typeof m.size === 'number' ? formatBytes(m.size) : ''}
                            </span>
                          </span>
                        </div>
                      );
                    })}
                  </div>
                </div>
              )}

              <div>
                <div className="text-content-muted uppercase tracking-wide text-[10px] mb-1">
                  {t('settings.localModel.status.expectedModels')}
                </div>
                <div className="space-y-1 text-xs">
                  <div className="flex items-center gap-2">
                    <span
                      className={
                        diagnostics.expected.chat_found
                          ? 'text-green-600 dark:text-green-300'
                          : 'text-red-600 dark:text-red-300'
                      }>
                      {diagnostics.expected.chat_found ? '✓' : '✗'}
                    </span>
                    <span className="text-content">
                      {t('settings.localModel.status.expectedChat').replace(
                        '{model}',
                        diagnostics.expected.chat_model
                      )}
                    </span>
                  </div>
                  <div className="flex items-center gap-2">
                    <span
                      className={
                        diagnostics.expected.embedding_found
                          ? 'text-green-600 dark:text-green-300'
                          : 'text-red-600 dark:text-red-300'
                      }>
                      {diagnostics.expected.embedding_found ? '✓' : '✗'}
                    </span>
                    <span className="text-content">
                      {t('settings.localModel.status.expectedEmbedding').replace(
                        '{model}',
                        diagnostics.expected.embedding_model
                      )}
                    </span>
                  </div>
                  <div className="flex items-center gap-2">
                    <span
                      className={
                        diagnostics.expected.vision_found
                          ? 'text-green-600 dark:text-green-300'
                          : 'text-amber-700 dark:text-amber-300'
                      }>
                      {diagnostics.expected.vision_found ? '✓' : '–'}
                    </span>
                    <span className="text-content">
                      {t('settings.localModel.status.expectedVision').replace(
                        '{model}',
                        diagnostics.expected.vision_model
                      )}
                    </span>
                  </div>
                </div>
              </div>

              {diagnostics.issues.length > 0 && (
                <div>
                  <div className="text-red-600 dark:text-red-300 uppercase tracking-wide text-[10px] mb-1">
                    {t('settings.localModel.status.issues')}
                  </div>
                  <ul className="space-y-1 text-xs text-red-600 dark:text-red-300">
                    {diagnostics.issues.map((issue, i) => (
                      <li key={i} className="flex gap-1.5">
                        <span className="shrink-0">&bull;</span>
                        <span>{issue}</span>
                      </li>
                    ))}
                  </ul>
                </div>
              )}

              <div className="text-xs text-content-muted">
                {t('settings.localModel.status.manageOllamaExternal')}
              </div>
            </>
          )}
        </div>
      </SettingsSection>
    </>
  );
};

export default ModelStatusSection;
