import { useEffect, useMemo, useState } from 'react';

import {
  formatBytes,
  formatEta,
  progressFromDownloads,
  progressFromStatus,
} from '../../../utils/localAiHelpers';
import {
  type ApplyPresetResult,
  type LocalAiAssetsStatus,
  type LocalAiDiagnostics,
  type LocalAiDownloadsProgress,
  type LocalAiEmbeddingResult,
  type LocalAiSpeechResult,
  type LocalAiStatus,
  type LocalAiSuggestion,
  type LocalAiTtsResult,
  openhumanLocalAiApplyPreset,
  openhumanLocalAiAssetsStatus,
  openhumanLocalAiDiagnostics,
  openhumanLocalAiDownload,
  openhumanLocalAiDownloadAllAssets,
  openhumanLocalAiDownloadAsset,
  openhumanLocalAiDownloadsProgress,
  openhumanLocalAiEmbed,
  openhumanLocalAiPresets,
  openhumanLocalAiPrompt,
  openhumanLocalAiSetOllamaPath,
  openhumanLocalAiStatus,
  openhumanLocalAiSuggestQuestions,
  openhumanLocalAiSummarize,
  openhumanLocalAiTranscribe,
  openhumanLocalAiTts,
  openhumanLocalAiVisionPrompt,
  type PresetsResponse,
} from '../../../utils/tauriCommands';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import DeviceCapabilitySection from './local-model/DeviceCapabilitySection';
import ModelDownloadSection from './local-model/ModelDownloadSection';
import ModelStatusSection from './local-model/ModelStatusSection';

const statusTone = (state: string): string => {
  switch (state) {
    case 'ready':
      return 'text-green-600';
    case 'downloading':
    case 'installing':
    case 'loading':
      return 'text-primary-600';
    case 'degraded':
      return 'text-amber-700';
    case 'disabled':
      return 'text-stone-500';
    default:
      return 'text-stone-700';
  }
};

const formatRamGb = (bytes: number): string => {
  const gb = bytes / (1024 * 1024 * 1024);
  return gb >= 10 ? `${Math.round(gb)} GB` : `${gb.toFixed(1)} GB`;
};

const LocalModelPanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const [status, setStatus] = useState<LocalAiStatus | null>(null);
  const [assets, setAssets] = useState<LocalAiAssetsStatus | null>(null);
  const [downloads, setDownloads] = useState<LocalAiDownloadsProgress | null>(null);
  const [statusError, setStatusError] = useState<string>('');
  const [isTriggeringDownload, setIsTriggeringDownload] = useState(false);
  const [bootstrapMessage, setBootstrapMessage] = useState<string>('');
  const [assetDownloadBusy, setAssetDownloadBusy] = useState<Record<string, boolean>>({});

  const [summaryInput, setSummaryInput] = useState('');
  const [summaryOutput, setSummaryOutput] = useState('');
  const [isSummaryLoading, setIsSummaryLoading] = useState(false);

  const [suggestInput, setSuggestInput] = useState('');
  const [suggestions, setSuggestions] = useState<LocalAiSuggestion[]>([]);
  const [isSuggestLoading, setIsSuggestLoading] = useState(false);

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

  const [presetsData, setPresetsData] = useState<PresetsResponse | null>(null);
  const [presetsLoading, setPresetsLoading] = useState(true);
  const [isApplyingPreset, setIsApplyingPreset] = useState(false);
  const [presetError, setPresetError] = useState('');
  const [presetSuccess, setPresetSuccess] = useState<ApplyPresetResult | null>(null);
  const [showAdvanced, setShowAdvanced] = useState(false);

  const progress = useMemo(() => {
    const downloadProgress = progressFromDownloads(downloads);
    if (downloadProgress != null) return downloadProgress;
    return progressFromStatus(status);
  }, [downloads, status]);
  const currentState = downloads?.state ?? status?.state;
  const isInstalling = currentState === 'installing';
  const isIndeterminateDownload =
    isInstalling ||
    (currentState === 'downloading' &&
      typeof downloads?.progress !== 'number' &&
      typeof status?.download_progress !== 'number');
  const isInstallError = status?.state === 'degraded' && status?.error_category === 'install';
  const [showErrorDetail, setShowErrorDetail] = useState(false);
  const [ollamaPathInput, setOllamaPathInput] = useState('');
  const [isSettingPath, setIsSettingPath] = useState(false);
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
      setStatusError('');
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to read local model status';
      setStatusError(message);
      setStatus(null);
      setAssets(null);
      setDownloads(null);
    }
  };

  const loadPresets = async () => {
    setPresetsLoading(true);
    try {
      const data = await openhumanLocalAiPresets();
      setPresetsData(data);
      setPresetError('');
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to load presets';
      console.warn('[LocalModelPanel] failed to load presets:', msg);
      setPresetError(msg);
    } finally {
      setPresetsLoading(false);
    }
  };

  const applyPreset = async (tier: string) => {
    setIsApplyingPreset(true);
    setPresetError('');
    setPresetSuccess(null);
    try {
      const result = await openhumanLocalAiApplyPreset(tier);
      setPresetSuccess(result);
      await loadPresets();
      await loadStatus();
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to apply preset';
      setPresetError(msg);
    } finally {
      setIsApplyingPreset(false);
    }
  };

  useEffect(() => {
    void loadStatus();
    void loadPresets();
    const timer = setInterval(() => {
      void loadStatus();
    }, 1500);
    return () => clearInterval(timer);
  }, []);

  const triggerDownload = async (force: boolean) => {
    setIsTriggeringDownload(true);
    setStatusError('');
    setBootstrapMessage('');
    try {
      await openhumanLocalAiDownload(force);
      await openhumanLocalAiDownloadAllAssets(force);
      const freshStatus = await openhumanLocalAiStatus();
      setStatus(freshStatus.result);
      if (freshStatus.result?.state === 'ready') {
        setBootstrapMessage(force ? 'Re-bootstrap complete' : 'Models verified');
      }
      setTimeout(() => setBootstrapMessage(''), 3000);
    } catch (err) {
      const message =
        err instanceof Error ? err.message : 'Failed to trigger local model bootstrap';
      setStatusError(message);
    } finally {
      setIsTriggeringDownload(false);
    }
  };

  const runSummaryTest = async () => {
    if (!summaryInput.trim()) return;
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

  const runSuggestTest = async () => {
    if (!suggestInput.trim()) return;
    setIsSuggestLoading(true);
    setSuggestions([]);
    setStatusError('');
    try {
      const result = await openhumanLocalAiSuggestQuestions(suggestInput.trim());
      setSuggestions(result.result);
      await loadStatus();
    } catch (err) {
      setStatusError(err instanceof Error ? err.message : 'Suggestion test failed');
    } finally {
      setIsSuggestLoading(false);
    }
  };

  const runPromptTest = async () => {
    if (!promptInput.trim()) return;
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
    if (!visionPromptInput.trim() || !visionImageInput.trim()) return;
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
    if (!embeddingInput.trim()) return;
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
    if (!audioPathInput.trim()) return;
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
    if (!ttsInput.trim()) return;
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

  const handleSetOllamaPath = async () => {
    setIsSettingPath(true);
    setStatusError('');
    try {
      await openhumanLocalAiSetOllamaPath(ollamaPathInput);
      await loadStatus();
    } catch (err) {
      setStatusError(err instanceof Error ? err.message : 'Failed to set Ollama path');
    } finally {
      setIsSettingPath(false);
    }
  };

  const handleClearOllamaPath = async () => {
    setOllamaPathInput('');
    setIsSettingPath(true);
    try {
      await openhumanLocalAiSetOllamaPath('');
      await loadStatus();
    } catch (err) {
      setStatusError(err instanceof Error ? err.message : 'Failed to clear Ollama path');
    } finally {
      setIsSettingPath(false);
    }
  };

  const handleRunDiagnostics = async () => {
    setIsDiagnosticsLoading(true);
    setDiagnosticsError('');
    try {
      const result = await openhumanLocalAiDiagnostics();
      setDiagnostics(result);
    } catch (err) {
      setDiagnosticsError(err instanceof Error ? err.message : 'Diagnostics failed');
    } finally {
      setIsDiagnosticsLoading(false);
    }
  };

  return (
    <div>
      <SettingsHeader
        title="Local Model"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-4">
        <DeviceCapabilitySection
          presetsData={presetsData}
          presetsLoading={presetsLoading}
          presetError={presetError}
          presetSuccess={presetSuccess}
          isApplyingPreset={isApplyingPreset}
          onApplyPreset={tier => void applyPreset(tier)}
          formatRamGb={formatRamGb}
        />

        <button
          type="button"
          onClick={() => setShowAdvanced(prev => !prev)}
          className="flex items-center gap-2 text-sm text-stone-500 hover:text-stone-700 transition-colors">
          <svg
            className={`w-4 h-4 transition-transform ${showAdvanced ? 'rotate-90' : ''}`}
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
          </svg>
          {showAdvanced ? 'Hide Advanced' : 'Show Advanced'}
        </button>

        {showAdvanced && (
          <>
            <ModelStatusSection
              status={status}
              downloads={downloads}
              diagnostics={diagnostics}
              isDiagnosticsLoading={isDiagnosticsLoading}
              diagnosticsError={diagnosticsError}
              statusError={statusError}
              isTriggeringDownload={isTriggeringDownload}
              bootstrapMessage={bootstrapMessage}
              progress={progress}
              isIndeterminateDownload={isIndeterminateDownload}
              isInstalling={isInstalling}
              isInstallError={isInstallError}
              showErrorDetail={showErrorDetail}
              ollamaPathInput={ollamaPathInput}
              isSettingPath={isSettingPath}
              downloadedText={downloadedText}
              speedText={speedText}
              etaText={etaText}
              statusTone={statusTone}
              onRefreshStatus={() => void loadStatus()}
              onTriggerDownload={force => void triggerDownload(force)}
              onSetOllamaPath={() => void handleSetOllamaPath()}
              onClearOllamaPath={() => void handleClearOllamaPath()}
              onSetOllamaPathInput={setOllamaPathInput}
              onToggleErrorDetail={() => setShowErrorDetail(v => !v)}
              onRunDiagnostics={() => void handleRunDiagnostics()}
            />

            <ModelDownloadSection
              assets={assets}
              assetDownloadBusy={assetDownloadBusy}
              statusTone={statusTone}
              onTriggerAssetDownload={capability => void triggerAssetDownload(capability)}
              summaryInput={summaryInput}
              summaryOutput={summaryOutput}
              isSummaryLoading={isSummaryLoading}
              onSetSummaryInput={setSummaryInput}
              onRunSummaryTest={() => void runSummaryTest()}
              suggestInput={suggestInput}
              suggestions={suggestions}
              isSuggestLoading={isSuggestLoading}
              onSetSuggestInput={setSuggestInput}
              onRunSuggestTest={() => void runSuggestTest()}
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
          </>
        )}
      </div>
    </div>
  );
};

export default LocalModelPanel;
