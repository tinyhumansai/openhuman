import { formatBytes, statusLabel } from '../../../../utils/localAiHelpers';
import type {
  LocalAiDiagnostics,
  LocalAiDownloadsProgress,
  LocalAiStatus,
  RepairAction,
} from '../../../../utils/tauriCommands';

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
  onRefreshStatus: () => void;
  onTriggerDownload: (force: boolean) => void;
  onSetOllamaPath: () => void;
  onClearOllamaPath: () => void;
  onSetOllamaPathInput: (value: string) => void;
  onToggleErrorDetail: () => void;
  onRunDiagnostics: () => void;
  onRepairAction?: (action: RepairAction) => void;
}

const repairActionLabel = (action: RepairAction): string => {
  switch (action.action) {
    case 'install_ollama':
      return 'Install Ollama';
    case 'start_server':
      return 'Start Server';
    case 'pull_model':
      return `Pull ${action.model}`;
  }
};

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
  onRefreshStatus,
  onTriggerDownload,
  onSetOllamaPath,
  onClearOllamaPath,
  onSetOllamaPathInput,
  onToggleErrorDetail,
  onRunDiagnostics,
  onRepairAction,
}: ModelStatusSectionProps) => {
  return (
    <>
      <section className="space-y-3">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-semibold text-stone-900">Runtime Status</h3>
          <button
            onClick={onRefreshStatus}
            className="text-sm text-primary-500 hover:text-primary-600 transition-colors">
            Refresh
          </button>
        </div>

        <div className="bg-stone-50 rounded-lg border border-stone-200 p-4 space-y-3">
          <div className="flex items-center justify-between text-sm">
            <span className="text-stone-500">State</span>
            <span className={`font-medium ${statusTone(status?.state ?? 'idle')}`}>
              {status ? statusLabel(downloads?.state ?? status.state) : 'Unavailable'}
            </span>
          </div>

          <div className="h-2 rounded-full bg-stone-200 overflow-hidden">
            <div
              className={`h-full bg-gradient-to-r from-blue-500 to-cyan-400 transition-all duration-500 ${
                isIndeterminateDownload ? 'animate-pulse' : ''
              }`}
              style={{ width: `${Math.round((isIndeterminateDownload ? 1 : progress) * 100)}%` }}
            />
          </div>

          <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-stone-500">
            <span>
              Progress:{' '}
              {isInstalling
                ? 'Installing Ollama runtime...'
                : isIndeterminateDownload
                  ? 'Downloading (size unknown)'
                  : `${Math.round(progress * 100)}%`}
            </span>
            {downloadedText && <span className="text-stone-600">{downloadedText}</span>}
            {speedText && <span className="text-primary-600">{speedText}</span>}
            {etaText && <span className="text-primary-500">ETA {etaText}</span>}
          </div>

          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 text-sm">
            <div className="rounded-md border border-stone-200 p-2">
              <div className="text-stone-500 text-xs uppercase tracking-wide">Provider</div>
              <div className="text-stone-800 mt-1">{status?.provider ?? 'n/a'}</div>
            </div>
            <div className="rounded-md border border-stone-200 p-2">
              <div className="text-stone-500 text-xs uppercase tracking-wide">Model</div>
              <div className="text-stone-800 mt-1">{status?.model_id ?? 'n/a'}</div>
            </div>
          </div>

          <div className="grid grid-cols-1 sm:grid-cols-3 gap-3 text-sm">
            <div className="rounded-md border border-stone-200 p-2">
              <div className="text-stone-500 text-xs uppercase tracking-wide">Backend</div>
              <div className="text-stone-800 mt-1">{status?.active_backend ?? 'cpu'}</div>
            </div>
            <div className="rounded-md border border-stone-200 p-2">
              <div className="text-stone-500 text-xs uppercase tracking-wide">Last Latency</div>
              <div className="text-stone-800 mt-1">
                {typeof status?.last_latency_ms === 'number'
                  ? `${status.last_latency_ms} ms`
                  : 'n/a'}
              </div>
            </div>
            <div className="rounded-md border border-stone-200 p-2">
              <div className="text-stone-500 text-xs uppercase tracking-wide">Generation TPS</div>
              <div className="text-stone-800 mt-1">
                {typeof status?.gen_toks_per_sec === 'number'
                  ? `${status.gen_toks_per_sec.toFixed(1)} tok/s`
                  : 'n/a'}
              </div>
            </div>
          </div>

          {status?.model_path && (
            <div className="text-xs text-stone-500 break-all">Artifact: {status.model_path}</div>
          )}

          {status?.backend_reason && (
            <div className="text-xs text-primary-600">{status.backend_reason}</div>
          )}
          {status?.warning && <div className="text-xs text-amber-700">{status.warning}</div>}
          {statusError && <div className="text-xs text-red-600">{statusError}</div>}

          {isInstallError && status?.error_detail && (
            <div className="space-y-1">
              <button
                onClick={onToggleErrorDetail}
                className="text-xs text-red-600 hover:text-red-500 underline">
                {showErrorDetail ? 'Hide error details' : 'Show error details'}
              </button>
              {showErrorDetail && (
                <pre className="max-h-40 overflow-auto rounded bg-red-50 border border-red-200 p-2 text-[10px] text-red-600 leading-tight whitespace-pre-wrap break-words">
                  {status.error_detail}
                </pre>
              )}
              <p className="text-xs text-stone-500">
                Install Ollama manually from{' '}
                <a
                  href="https://ollama.com"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-primary-500 hover:text-primary-600 underline">
                  ollama.com
                </a>{' '}
                then set its path below.
              </p>
            </div>
          )}

          <div className="space-y-1">
            <div className="text-stone-500 text-xs uppercase tracking-wide">
              Ollama Binary Path (optional)
            </div>
            <div className="flex items-center gap-2">
              <input
                type="text"
                value={ollamaPathInput}
                onChange={e => onSetOllamaPathInput(e.target.value)}
                placeholder="/usr/local/bin/ollama"
                className="flex-1 rounded-md border border-stone-200 bg-white px-2 py-1.5 text-xs text-stone-900 placeholder:text-stone-400 focus:border-primary-500 focus:outline-none"
              />
              <button
                onClick={onSetOllamaPath}
                disabled={isSettingPath || !ollamaPathInput.trim()}
                className="px-2 py-1.5 text-xs rounded-md bg-primary-600 hover:bg-primary-700 disabled:opacity-60 text-white whitespace-nowrap">
                {isSettingPath ? 'Setting...' : 'Set Path'}
              </button>
              {ollamaPathInput && (
                <button
                  onClick={onClearOllamaPath}
                  disabled={isSettingPath}
                  className="px-2 py-1.5 text-xs rounded-md border border-stone-200 hover:border-stone-300 disabled:opacity-60 text-stone-600 whitespace-nowrap">
                  Clear
                </button>
              )}
            </div>
          </div>

          <div className="flex items-center gap-2 pt-1">
            {status?.state === 'ready' ? (
              <span className="inline-flex items-center gap-1 px-3 py-1.5 text-xs rounded-md bg-green-50 text-green-700 border border-green-200 font-medium">
                <svg className="h-3 w-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M5 13l4 4L19 7"
                  />
                </svg>
                Running
              </span>
            ) : (
              <button
                onClick={() => onTriggerDownload(false)}
                disabled={isTriggeringDownload}
                className="px-3 py-1.5 text-xs rounded-md bg-primary-600 hover:bg-primary-700 disabled:opacity-60 text-white">
                {isTriggeringDownload
                  ? 'Triggering...'
                  : status?.state === 'degraded'
                    ? 'Retry Bootstrap'
                    : 'Bootstrap / Resume'}
              </button>
            )}
            <button
              onClick={() => onTriggerDownload(true)}
              disabled={isTriggeringDownload}
              className="px-3 py-1.5 text-xs rounded-md border border-stone-200 hover:border-stone-300 disabled:opacity-60 text-stone-600">
              {isTriggeringDownload ? 'Working...' : 'Force Re-bootstrap'}
            </button>
            {bootstrapMessage && <span className="text-xs text-green-600">{bootstrapMessage}</span>}
          </div>
        </div>
      </section>

      <section className="space-y-3">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-semibold text-stone-900">Ollama Diagnostics</h3>
          <button
            onClick={onRunDiagnostics}
            disabled={isDiagnosticsLoading}
            className="px-3 py-1.5 text-xs rounded-md bg-primary-600 hover:bg-primary-700 disabled:opacity-60 text-white">
            {isDiagnosticsLoading ? 'Checking...' : 'Run Diagnostics'}
          </button>
        </div>
        <div className="bg-stone-50 rounded-lg border border-stone-200 p-4 space-y-3">
          {!diagnostics && !diagnosticsError && (
            <p className="text-xs text-stone-500">
              Click &ldquo;Run Diagnostics&rdquo; to verify Ollama is running and models are
              installed.
            </p>
          )}
          {isDiagnosticsLoading && (
            <div className="flex items-center gap-2 text-xs text-primary-600">
              <div className="h-3 w-3 rounded-full border-2 border-blue-400 border-t-transparent animate-spin" />
              Checking Ollama server and models...
            </div>
          )}
          {diagnosticsError && (
            <div className="rounded-md bg-red-50 border border-red-300 p-3 text-xs text-red-600">
              {diagnosticsError}
            </div>
          )}
          {diagnostics && (
            <>
              <div className="flex items-center gap-2 text-sm">
                <span
                  className={`inline-block h-2.5 w-2.5 rounded-full ${diagnostics.ok ? 'bg-green-400' : 'bg-red-400'}`}
                />
                <span className={diagnostics.ok ? 'text-green-600' : 'text-red-600'}>
                  {diagnostics.ok
                    ? 'All checks passed'
                    : `${diagnostics.issues.length} issue(s) found`}
                </span>
              </div>

              <div className="grid grid-cols-2 gap-2 text-xs">
                <div className="rounded-md border border-stone-200 p-2">
                  <div className="text-stone-400 uppercase tracking-wide text-[10px]">Server</div>
                  <div
                    className={`mt-1 font-medium ${diagnostics.ollama_running ? 'text-green-600' : 'text-red-600'}`}>
                    {diagnostics.ollama_running ? 'Running' : 'Not running'}
                  </div>
                  {diagnostics.ollama_base_url && (
                    <div
                      className="mt-0.5 text-stone-400 truncate text-[10px]"
                      title={diagnostics.ollama_base_url}>
                      {diagnostics.ollama_base_url}
                    </div>
                  )}
                </div>
                <div className="rounded-md border border-stone-200 p-2">
                  <div className="text-stone-400 uppercase tracking-wide text-[10px]">Binary</div>
                  <div
                    className="mt-1 text-stone-600 truncate"
                    title={
                      diagnostics.ollama_binary_path ??
                      (diagnostics.ollama_running ? 'External process' : 'Not found')
                    }>
                    {diagnostics.ollama_binary_path === null
                      ? diagnostics.ollama_running
                        ? 'Running via external process'
                        : 'Not found'
                      : diagnostics.ollama_binary_path}
                  </div>
                </div>
              </div>

              {diagnostics.installed_models.length > 0 && (
                <div>
                  <div className="text-stone-400 uppercase tracking-wide text-[10px] mb-1">
                    Installed Models ({diagnostics.installed_models.length})
                  </div>
                  <div className="space-y-1">
                    {diagnostics.installed_models.map(m => (
                      <div
                        key={m.name}
                        className="flex items-center justify-between rounded border border-stone-200 px-2 py-1.5 text-xs">
                        <span className="text-stone-800 font-medium">{m.name}</span>
                        <span className="text-stone-400">
                          {typeof m.size === 'number' ? formatBytes(m.size) : ''}
                        </span>
                      </div>
                    ))}
                  </div>
                </div>
              )}

              <div>
                <div className="text-stone-400 uppercase tracking-wide text-[10px] mb-1">
                  Expected Models
                </div>
                <div className="space-y-1 text-xs">
                  <div className="flex items-center gap-2">
                    <span
                      className={
                        diagnostics.expected.chat_found ? 'text-green-600' : 'text-red-600'
                      }>
                      {diagnostics.expected.chat_found ? '\u2713' : '\u2717'}
                    </span>
                    <span className="text-stone-700">Chat: {diagnostics.expected.chat_model}</span>
                  </div>
                  <div className="flex items-center gap-2">
                    <span
                      className={
                        diagnostics.expected.embedding_found ? 'text-green-600' : 'text-red-600'
                      }>
                      {diagnostics.expected.embedding_found ? '\u2713' : '\u2717'}
                    </span>
                    <span className="text-stone-700">
                      Embedding: {diagnostics.expected.embedding_model}
                    </span>
                  </div>
                  <div className="flex items-center gap-2">
                    <span
                      className={
                        diagnostics.expected.vision_found ? 'text-green-600' : 'text-amber-700'
                      }>
                      {diagnostics.expected.vision_found ? '\u2713' : '\u2013'}
                    </span>
                    <span className="text-stone-700">
                      Vision: {diagnostics.expected.vision_model}
                    </span>
                  </div>
                </div>
              </div>

              {diagnostics.issues.length > 0 && (
                <div>
                  <div className="text-red-600 uppercase tracking-wide text-[10px] mb-1">
                    Issues
                  </div>
                  <ul className="space-y-1 text-xs text-red-600">
                    {diagnostics.issues.map((issue, i) => (
                      <li key={i} className="flex gap-1.5">
                        <span className="shrink-0">&bull;</span>
                        <span>{issue}</span>
                      </li>
                    ))}
                  </ul>
                </div>
              )}

              {diagnostics.repair_actions && diagnostics.repair_actions.length > 0 && (
                <div>
                  <div className="text-amber-700 uppercase tracking-wide text-[10px] mb-1">
                    Suggested Fixes
                  </div>
                  <div className="flex flex-wrap gap-2">
                    {diagnostics.repair_actions.map((action, i) => (
                      <button
                        key={i}
                        onClick={() => onRepairAction?.(action)}
                        className="px-2.5 py-1 text-xs rounded-md bg-amber-50 border border-amber-300 text-amber-800 hover:bg-amber-100 transition-colors">
                        {repairActionLabel(action)}
                      </button>
                    ))}
                  </div>
                </div>
              )}
            </>
          )}
        </div>
      </section>
    </>
  );
};

export default ModelStatusSection;
