import debug from 'debug';
import { useEffect, useMemo, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  formatBytes,
  formatEta,
  progressFromDownloads,
  progressFromStatus,
} from '../../../utils/localAiHelpers';
import {
  type LocalAiAssetsStatus,
  type LocalAiDiagnostics,
  type LocalAiDownloadsProgress,
  type LocalAiEmbeddingResult,
  type LocalAiSpeechResult,
  type LocalAiStatus,
  type LocalAiTtsResult,
  type OllamaConnectionTestResult,
  openhumanLocalAiAssetsStatus,
  openhumanLocalAiDiagnostics,
  openhumanLocalAiDownloadAsset,
  openhumanLocalAiDownloadsProgress,
  openhumanLocalAiEmbed,
  openhumanLocalAiPrompt,
  openhumanLocalAiStatus,
  openhumanLocalAiSummarize,
  openhumanLocalAiTestConnection,
  openhumanLocalAiTranscribe,
  openhumanLocalAiTts,
  openhumanLocalAiVisionPrompt,
  openhumanUpdateLocalAiSettings,
} from '../../../utils/tauriCommands';
import { openhumanGetConfig } from '../../../utils/tauriCommands/config';
import SettingsPanel from '../layout/SettingsPanel';
import ModelDownloadSection from './local-model/ModelDownloadSection';
import ModelStatusSection from './local-model/ModelStatusSection';

const log = debug('openhuman:local-model-debug');

const statusTone = (state: string): string => {
  switch (state) {
    case 'ready':
      return 'text-green-600 dark:text-green-300';
    case 'downloading':
    case 'installing':
    case 'loading':
      return 'text-primary-600 dark:text-primary-300';
    case 'degraded':
      return 'text-amber-700 dark:text-amber-300';
    case 'disabled':
      return 'text-content-muted';
    default:
      return 'text-content-secondary';
  }
};

const LocalModelDebugPanel = () => {
  const { t } = useT();

  const [status, setStatus] = useState<LocalAiStatus | null>(null);
  const [assets, setAssets] = useState<LocalAiAssetsStatus | null>(null);
  const [downloads, setDownloads] = useState<LocalAiDownloadsProgress | null>(null);
  const [statusError, setStatusError] = useState<string>('');
  const [assetDownloadBusy, setAssetDownloadBusy] = useState<Record<string, boolean>>({});

  const [summaryInput, setSummaryInput] = useState('');
  const [summaryOutput, setSummaryOutput] = useState('');
  const [isSummaryLoading, setIsSummaryLoading] = useState(false);

  const [promptInput, setPromptInput] = useState('');
  const [promptOutput, setPromptOutput] = useState('');
  const [isPromptLoading, setIsPromptLoading] = useState(false);
  const [promptNoThink, setPromptNoThink] = useState(true);
  const [promptError, setPromptError] = useState('');

  const [visionPromptInput, setVisionPromptInput] = useState('');
  const [visionImageInput, setVisionImageInput] = useState('');
  const [visionOutput, setVisionOutput] = useState('');
  const [isVisionLoading, setIsVisionLoading] = useState(false);

  const [embeddingInput, setEmbeddingInput] = useState('');
  const [embeddingOutput, setEmbeddingOutput] = useState<LocalAiEmbeddingResult | null>(null);
  const [isEmbeddingLoading, setIsEmbeddingLoading] = useState(false);

  const [audioPathInput, setAudioPathInput] = useState('');
  const [transcribeOutput, setTranscribeOutput] = useState<LocalAiSpeechResult | null>(null);
  const [isTranscribeLoading, setIsTranscribeLoading] = useState(false);

  const [ttsInput, setTtsInput] = useState('');
  const [ttsOutputPath, setTtsOutputPath] = useState('');
  const [ttsOutput, setTtsOutput] = useState<LocalAiTtsResult | null>(null);
  const [isTtsLoading, setIsTtsLoading] = useState(false);

  const [diagnostics, setDiagnostics] = useState<LocalAiDiagnostics | null>(null);
  const [isDiagnosticsLoading, setIsDiagnosticsLoading] = useState(false);
  const [diagnosticsError, setDiagnosticsError] = useState('');

  const [showErrorDetail, setShowErrorDetail] = useState(false);

  const DEFAULT_OLLAMA_URL = 'http://localhost:11434';
  const [ollamaBaseUrlInput, setOllamaBaseUrlInput] = useState(DEFAULT_OLLAMA_URL);
  const [savedOllamaBaseUrl, setSavedOllamaBaseUrl] = useState(DEFAULT_OLLAMA_URL);
  const [isTestingConnection, setIsTestingConnection] = useState(false);
  const [connectionTestResult, setConnectionTestResult] =
    useState<OllamaConnectionTestResult | null>(null);
  const [isSavingUrl, setIsSavingUrl] = useState(false);

  const progress = useMemo(() => {
    const downloadProgress = progressFromDownloads(downloads);
    if (downloadProgress != null) return downloadProgress;
    return progressFromStatus(status);
  }, [downloads, status]);

  const runtimeEnabled = status?.state !== 'disabled';
  const currentState = downloads?.state ?? status?.state;
  const isInstalling = currentState === 'installing';
  const isIndeterminateDownload =
    isInstalling ||
    (currentState === 'downloading' &&
      typeof downloads?.progress !== 'number' &&
      typeof status?.download_progress !== 'number');
  const isInstallError = status?.state === 'degraded' && status?.error_category === 'install';

  const downloadedBytes = downloads?.downloaded_bytes ?? status?.downloaded_bytes;
  const totalBytes = downloads?.total_bytes ?? status?.total_bytes;
  const speedBps = downloads?.speed_bps ?? status?.download_speed_bps;
  const etaSeconds = downloads?.eta_seconds ?? status?.eta_seconds;

  const downloadedText =
    typeof downloadedBytes === 'number'
      ? `${formatBytes(downloadedBytes)}${typeof totalBytes === 'number' ? ` / ${formatBytes(totalBytes)}` : ''}`
      : '';
  const speedText =
    typeof speedBps === 'number' && speedBps > 0 ? `${formatBytes(speedBps)}/s` : '';
  const etaText = formatEta(etaSeconds);

  const loadStatus = async () => {
    try {
      const [statusResponse, assetsResponse, downloadsResponse] = await Promise.all([
        openhumanLocalAiStatus(),
        openhumanLocalAiAssetsStatus(),
        openhumanLocalAiDownloadsProgress(),
      ]);
      setStatus(statusResponse.result);
      setAssets(assetsResponse.result);
      setDownloads(downloadsResponse.result);
    } catch {
      // Poll failures are non-critical — don't clear action errors.
      // Status/assets/downloads retain their last known values.
    }
  };

  useEffect(() => {
    const initialLoad = window.setTimeout(() => {
      void loadStatus();
    }, 0);
    const timer = window.setInterval(() => {
      void loadStatus();
    }, 1500);
    return () => {
      window.clearTimeout(initialLoad);
      window.clearInterval(timer);
    };
  }, []);

  useEffect(() => {
    const seedSavedUrl = async () => {
      try {
        const configResponse = await openhumanGetConfig();
        const localAi = configResponse.result?.config?.local_ai as
          | Record<string, unknown>
          | undefined;
        const saved = localAi?.base_url as string | undefined | null;
        if (saved && saved.trim()) {
          setOllamaBaseUrlInput(saved.trim());
          setSavedOllamaBaseUrl(saved.trim());
        }
      } catch {
        // Non-critical — stay on default.
      }
    };
    void seedSavedUrl();
  }, []);

  const runSummaryTest = async () => {
    if (!runtimeEnabled || !summaryInput.trim()) return;
    setIsSummaryLoading(true);
    setSummaryOutput('');
    setStatusError('');
    try {
      const result = await openhumanLocalAiSummarize(summaryInput.trim(), 220);
      setSummaryOutput(result.result);
      await loadStatus();
    } catch (err) {
      setStatusError(err instanceof Error ? err.message : 'Summarization test failed');
    } finally {
      setIsSummaryLoading(false);
    }
  };

  const runPromptTest = async () => {
    if (!runtimeEnabled || !promptInput.trim()) return;
    setIsPromptLoading(true);
    setPromptOutput('');
    setPromptError('');
    try {
      const result = await openhumanLocalAiPrompt(promptInput.trim(), 180, promptNoThink);
      setPromptOutput(result.result);
      await loadStatus();
    } catch (err) {
      setPromptError(err instanceof Error ? err.message : 'Prompt test failed');
    } finally {
      setIsPromptLoading(false);
    }
  };

  const runVisionTest = async () => {
    if (!runtimeEnabled || !visionPromptInput.trim() || !visionImageInput.trim()) return;
    setIsVisionLoading(true);
    setVisionOutput('');
    setStatusError('');
    try {
      const imageRefs = visionImageInput
        .split('\n')
        .map(v => v.trim())
        .filter(Boolean);
      const result = await openhumanLocalAiVisionPrompt(visionPromptInput.trim(), imageRefs, 220);
      setVisionOutput(result.result);
      await loadStatus();
    } catch (err) {
      setStatusError(err instanceof Error ? err.message : 'Vision test failed');
    } finally {
      setIsVisionLoading(false);
    }
  };

  const runEmbeddingTest = async () => {
    if (!runtimeEnabled || !embeddingInput.trim()) return;
    setIsEmbeddingLoading(true);
    setEmbeddingOutput(null);
    setStatusError('');
    try {
      const inputs = embeddingInput
        .split('\n')
        .map(v => v.trim())
        .filter(Boolean);
      const result = await openhumanLocalAiEmbed(inputs);
      setEmbeddingOutput(result.result);
      await loadStatus();
    } catch (err) {
      setStatusError(err instanceof Error ? err.message : 'Embedding test failed');
    } finally {
      setIsEmbeddingLoading(false);
    }
  };

  const runTranscribeTest = async () => {
    if (!runtimeEnabled || !audioPathInput.trim()) return;
    setIsTranscribeLoading(true);
    setTranscribeOutput(null);
    setStatusError('');
    try {
      const result = await openhumanLocalAiTranscribe(audioPathInput.trim());
      setTranscribeOutput(result.result);
      await loadStatus();
    } catch (err) {
      setStatusError(err instanceof Error ? err.message : 'Transcription test failed');
    } finally {
      setIsTranscribeLoading(false);
    }
  };

  const runTtsTest = async () => {
    if (!runtimeEnabled || !ttsInput.trim()) return;
    setIsTtsLoading(true);
    setTtsOutput(null);
    setStatusError('');
    try {
      const result = await openhumanLocalAiTts(
        ttsInput.trim(),
        ttsOutputPath.trim() ? ttsOutputPath.trim() : undefined
      );
      setTtsOutput(result.result);
      await loadStatus();
    } catch (err) {
      setStatusError(err instanceof Error ? err.message : 'TTS test failed');
    } finally {
      setIsTtsLoading(false);
    }
  };

  const triggerAssetDownload = async (
    capability: 'chat' | 'vision' | 'embedding' | 'stt' | 'tts'
  ) => {
    if (!runtimeEnabled) return;
    setAssetDownloadBusy(prev => ({ ...prev, [capability]: true }));
    setStatusError('');
    try {
      const result = await openhumanLocalAiDownloadAsset(capability);
      setAssets(result.result);
      await loadStatus();
    } catch (err) {
      setStatusError(err instanceof Error ? err.message : `Failed to download ${capability} asset`);
    } finally {
      setAssetDownloadBusy(prev => ({ ...prev, [capability]: false }));
    }
  };

  const handleRunDiagnostics = async () => {
    setIsDiagnosticsLoading(true);
    setDiagnosticsError('');
    try {
      const result = await openhumanLocalAiDiagnostics();
      setDiagnostics(result);
      if (result.ollama_base_url) {
        const reported = result.ollama_base_url;
        // Only overwrite the input if the user hasn't made unsaved edits.
        setOllamaBaseUrlInput(prev => (prev === savedOllamaBaseUrl ? reported : prev));
        setSavedOllamaBaseUrl(reported);
      }
    } catch (err) {
      setDiagnosticsError(err instanceof Error ? err.message : 'Diagnostics failed');
    } finally {
      setIsDiagnosticsLoading(false);
    }
  };

  const handleTestConnection = async () => {
    setIsTestingConnection(true);
    setConnectionTestResult(null);
    try {
      const result = await openhumanLocalAiTestConnection(ollamaBaseUrlInput);
      log('[local_ai:ui] test_connection result: reachable=%o', result.reachable);
      setConnectionTestResult(result);
    } catch (err) {
      setConnectionTestResult({
        reachable: false,
        error: err instanceof Error ? err.message : 'Connection test failed',
        models_count: null,
      });
    } finally {
      setIsTestingConnection(false);
    }
  };

  const handleSaveOllamaBaseUrl = async () => {
    setIsSavingUrl(true);
    try {
      await openhumanUpdateLocalAiSettings({ base_url: ollamaBaseUrlInput });
      log(
        '[local_ai:ui] saved ollama base_url=%s',
        ollamaBaseUrlInput.replace(/\/\/[^@]*@/, '//***@')
      );
      setSavedOllamaBaseUrl(ollamaBaseUrlInput);
    } catch (err) {
      setStatusError(err instanceof Error ? err.message : 'Failed to save URL');
    } finally {
      setIsSavingUrl(false);
    }
  };

  const handleResetOllamaBaseUrl = async () => {
    setOllamaBaseUrlInput(DEFAULT_OLLAMA_URL);
    setConnectionTestResult(null);
    setIsSavingUrl(true);
    try {
      await openhumanUpdateLocalAiSettings({ base_url: null });
      log('[local_ai:ui] reset ollama base_url to default');
      setSavedOllamaBaseUrl(DEFAULT_OLLAMA_URL);
    } catch (err) {
      setStatusError(err instanceof Error ? err.message : 'Failed to reset URL');
    } finally {
      setIsSavingUrl(false);
    }
  };

  return (
    <SettingsPanel description={t('settings.developerMenu.localModelDebug.desc')}>
      <ModelStatusSection
        status={status}
        downloads={downloads}
        diagnostics={diagnostics}
        isDiagnosticsLoading={isDiagnosticsLoading}
        diagnosticsError={diagnosticsError}
        statusError={statusError}
        isTriggeringDownload={false}
        bootstrapMessage=""
        progress={progress}
        isIndeterminateDownload={isIndeterminateDownload}
        isInstalling={isInstalling}
        isInstallError={isInstallError}
        showErrorDetail={showErrorDetail}
        ollamaPathInput=""
        isSettingPath={false}
        downloadedText={downloadedText}
        speedText={speedText}
        etaText={etaText}
        statusTone={statusTone}
        runtimeEnabled={runtimeEnabled}
        ollamaBaseUrlInput={ollamaBaseUrlInput}
        isTestingConnection={isTestingConnection}
        connectionTestResult={connectionTestResult}
        isSavingUrl={isSavingUrl}
        savedOllamaBaseUrl={savedOllamaBaseUrl}
        onRefreshStatus={() => void loadStatus()}
        onTriggerDownload={() => {}}
        onSetOllamaPath={() => {}}
        onClearOllamaPath={() => {}}
        onSetOllamaPathInput={() => {}}
        onToggleErrorDetail={() => setShowErrorDetail(v => !v)}
        onRunDiagnostics={() => void handleRunDiagnostics()}
        onSetOllamaBaseUrlInput={setOllamaBaseUrlInput}
        onTestConnection={() => void handleTestConnection()}
        onSaveOllamaBaseUrl={() => void handleSaveOllamaBaseUrl()}
        onResetOllamaBaseUrl={() => void handleResetOllamaBaseUrl()}
      />

      <ModelDownloadSection
        assets={assets}
        assetDownloadBusy={assetDownloadBusy}
        statusTone={statusTone}
        runtimeEnabled={runtimeEnabled}
        onTriggerAssetDownload={capability => void triggerAssetDownload(capability)}
        summaryInput={summaryInput}
        summaryOutput={summaryOutput}
        isSummaryLoading={isSummaryLoading}
        onSetSummaryInput={setSummaryInput}
        onRunSummaryTest={() => void runSummaryTest()}
        promptInput={promptInput}
        promptOutput={promptOutput}
        promptError={promptError}
        isPromptLoading={isPromptLoading}
        promptNoThink={promptNoThink}
        onSetPromptInput={setPromptInput}
        onSetPromptNoThink={setPromptNoThink}
        onRunPromptTest={() => void runPromptTest()}
        visionPromptInput={visionPromptInput}
        visionImageInput={visionImageInput}
        visionOutput={visionOutput}
        isVisionLoading={isVisionLoading}
        onSetVisionPromptInput={setVisionPromptInput}
        onSetVisionImageInput={setVisionImageInput}
        onRunVisionTest={() => void runVisionTest()}
        embeddingInput={embeddingInput}
        embeddingOutput={embeddingOutput}
        isEmbeddingLoading={isEmbeddingLoading}
        onSetEmbeddingInput={setEmbeddingInput}
        onRunEmbeddingTest={() => void runEmbeddingTest()}
        audioPathInput={audioPathInput}
        transcribeOutput={transcribeOutput}
        isTranscribeLoading={isTranscribeLoading}
        onSetAudioPathInput={setAudioPathInput}
        onRunTranscribeTest={() => void runTranscribeTest()}
        ttsInput={ttsInput}
        ttsOutputPath={ttsOutputPath}
        ttsOutput={ttsOutput}
        isTtsLoading={isTtsLoading}
        onSetTtsInput={setTtsInput}
        onSetTtsOutputPath={setTtsOutputPath}
        onRunTtsTest={() => void runTtsTest()}
      />
    </SettingsPanel>
  );
};

export default LocalModelDebugPanel;
