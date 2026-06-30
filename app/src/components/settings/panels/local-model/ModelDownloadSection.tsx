import { useT } from '../../../../lib/i18n/I18nContext';
import { statusLabel } from '../../../../utils/localAiHelpers';
import type {
  LocalAiAssetsStatus,
  LocalAiEmbeddingResult,
  LocalAiSpeechResult,
  LocalAiTtsResult,
} from '../../../../utils/tauriCommands';
import Button from '../../../ui/Button';
import {
  SettingsSection,
  SettingsStatusLine,
  SettingsTextArea,
  SettingsTextField,
} from '../../controls';

interface ModelDownloadSectionProps {
  assets: LocalAiAssetsStatus | null;
  assetDownloadBusy: Record<string, boolean>;
  statusTone: (state: string) => string;
  runtimeEnabled: boolean;
  onTriggerAssetDownload: (capability: 'chat' | 'vision' | 'embedding' | 'stt' | 'tts') => void;

  summaryInput: string;
  summaryOutput: string;
  isSummaryLoading: boolean;
  onSetSummaryInput: (value: string) => void;
  onRunSummaryTest: () => void;

  promptInput: string;
  promptOutput: string;
  promptError: string;
  isPromptLoading: boolean;
  promptNoThink: boolean;
  onSetPromptInput: (value: string) => void;
  onSetPromptNoThink: (value: boolean) => void;
  onRunPromptTest: () => void;

  visionPromptInput: string;
  visionImageInput: string;
  visionOutput: string;
  isVisionLoading: boolean;
  onSetVisionPromptInput: (value: string) => void;
  onSetVisionImageInput: (value: string) => void;
  onRunVisionTest: () => void;

  embeddingInput: string;
  embeddingOutput: LocalAiEmbeddingResult | null;
  isEmbeddingLoading: boolean;
  onSetEmbeddingInput: (value: string) => void;
  onRunEmbeddingTest: () => void;

  audioPathInput: string;
  transcribeOutput: LocalAiSpeechResult | null;
  isTranscribeLoading: boolean;
  onSetAudioPathInput: (value: string) => void;
  onRunTranscribeTest: () => void;

  ttsInput: string;
  ttsOutputPath: string;
  ttsOutput: LocalAiTtsResult | null;
  isTtsLoading: boolean;
  onSetTtsInput: (value: string) => void;
  onSetTtsOutputPath: (value: string) => void;
  onRunTtsTest: () => void;
}

const ModelDownloadSection = ({
  assets,
  assetDownloadBusy,
  statusTone,
  runtimeEnabled,
  onTriggerAssetDownload,
  summaryInput,
  summaryOutput,
  isSummaryLoading,
  onSetSummaryInput,
  onRunSummaryTest,
  promptInput,
  promptOutput,
  promptError,
  isPromptLoading,
  promptNoThink,
  onSetPromptInput,
  onSetPromptNoThink,
  onRunPromptTest,
  visionPromptInput,
  visionImageInput,
  visionOutput,
  isVisionLoading,
  onSetVisionPromptInput,
  onSetVisionImageInput,
  onRunVisionTest,
  embeddingInput,
  embeddingOutput,
  isEmbeddingLoading,
  onSetEmbeddingInput,
  onRunEmbeddingTest,
  audioPathInput,
  transcribeOutput,
  isTranscribeLoading,
  onSetAudioPathInput,
  onRunTranscribeTest,
  ttsInput,
  ttsOutputPath,
  ttsOutput,
  isTtsLoading,
  onSetTtsInput,
  onSetTtsOutputPath,
  onRunTtsTest,
}: ModelDownloadSectionProps) => {
  const { t } = useT();
  const capabilityCards = [
    ['settings.localModel.download.capabilityChat', 'chat', assets?.chat],
    ['settings.localModel.download.capabilityVision', 'vision', assets?.vision],
    ['settings.localModel.download.capabilityEmbedding', 'embedding', assets?.embedding],
    ['settings.localModel.download.capabilityStt', 'stt', assets?.stt],
    ['settings.localModel.download.capabilityTts', 'tts', assets?.tts],
  ] as const;

  return (
    <>
      <SettingsSection title={t('settings.localModel.download.capabilityAssets')}>
        <div className="px-4 py-3 space-y-3">
          <div className="text-xs text-content-muted">
            {t('settings.localModel.download.quantizationPref')} {assets?.quantization ?? 'q4'}
          </div>
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 text-sm">
            {capabilityCards.map(([labelKey, key, item]) => (
              <div key={key} className="rounded-md border border-line p-2">
                <div className="text-content-muted text-xs uppercase tracking-wide">
                  {t(labelKey)}
                </div>
                <div className="text-content mt-1 break-all">
                  {item?.id ?? t('settings.localModel.download.notAvailable')}
                </div>
                <div className={`text-xs mt-1 ${statusTone(item?.state ?? 'idle')}`}>
                  {statusLabel(item?.state ?? 'idle')}
                </div>
                {item?.path && (
                  <div className="text-[10px] text-content-muted mt-1 break-all">{item.path}</div>
                )}
                {item?.provider === 'ollama' || item?.provider === 'lm_studio' ? (
                  <div className="mt-2 text-[10px] text-content-muted">
                    {t('settings.localModel.download.manageExternal')}
                  </div>
                ) : (
                  <Button
                    type="button"
                    variant="secondary"
                    size="xs"
                    className="mt-2"
                    onClick={() => onTriggerAssetDownload(key)}
                    disabled={!runtimeEnabled || assetDownloadBusy[key]}>
                    {assetDownloadBusy[key]
                      ? t('settings.localModel.download.downloading')
                      : t('common.download')}
                  </Button>
                )}
              </div>
            ))}
          </div>
        </div>
      </SettingsSection>

      <SettingsSection title={t('settings.localModel.download.testSummarization')}>
        <div className="px-4 py-3 space-y-3">
          <SettingsTextArea
            value={summaryInput}
            onChange={e => onSetSummaryInput(e.target.value)}
            placeholder={t('settings.localModel.download.summarizePlaceholder')}
            rows={4}
            aria-label={t('settings.localModel.download.testSummarization')}
          />
          <div className="flex items-center justify-between">
            <div className="text-xs text-content-muted">
              {t('settings.localModel.download.summaryHelper')}
            </div>
            <Button
              type="button"
              variant="secondary"
              size="xs"
              onClick={onRunSummaryTest}
              disabled={!runtimeEnabled || isSummaryLoading || !summaryInput.trim()}>
              {isSummaryLoading
                ? t('settings.localModel.download.running')
                : t('settings.localModel.download.runSummaryTest')}
            </Button>
          </div>
          {summaryOutput && (
            <pre className="whitespace-pre-wrap rounded-md bg-surface-muted border border-line p-3 text-xs text-content-secondary">
              {summaryOutput}
            </pre>
          )}
        </div>
      </SettingsSection>

      <SettingsSection title={t('settings.localModel.download.testCustomPrompt')}>
        <div className="px-4 py-3 space-y-3">
          <SettingsTextArea
            value={promptInput}
            onChange={e => onSetPromptInput(e.target.value)}
            placeholder={t('settings.localModel.download.promptPlaceholder')}
            rows={4}
            aria-label={t('settings.localModel.download.testCustomPrompt')}
          />
          <div className="flex flex-wrap items-center justify-between gap-2">
            <label className="flex items-center gap-2 text-xs text-content-secondary">
              <input
                type="checkbox"
                checked={promptNoThink}
                onChange={e => onSetPromptNoThink(e.target.checked)}
                className="h-3.5 w-3.5 rounded border-line-strong bg-surface text-primary-500 focus:ring-primary-500"
              />
              {t('settings.localModel.download.noThinkMode')}
            </label>
            <Button
              type="button"
              variant="secondary"
              size="xs"
              onClick={onRunPromptTest}
              disabled={!runtimeEnabled || isPromptLoading || !promptInput.trim()}>
              {isPromptLoading
                ? t('settings.localModel.download.running')
                : t('settings.localModel.download.runPromptTest')}
            </Button>
          </div>
          {isPromptLoading && (
            <div className="flex items-center gap-2 text-xs text-primary-600 dark:text-primary-300">
              <div className="h-3 w-3 rounded-full border-2 border-blue-400 border-t-transparent animate-spin" />
              {t('settings.localModel.download.runningPrompt')}
            </div>
          )}
          {promptError && <SettingsStatusLine saving={false} error={promptError} savingLabel="" />}
          {promptOutput && (
            <pre className="whitespace-pre-wrap rounded-md bg-surface-muted border border-line p-3 text-xs text-content-secondary max-h-64 overflow-auto">
              {promptOutput}
            </pre>
          )}
        </div>
      </SettingsSection>

      <SettingsSection title={t('settings.localModel.download.testVisionPrompt')}>
        <div className="px-4 py-3 space-y-3">
          <SettingsTextArea
            value={visionPromptInput}
            onChange={e => onSetVisionPromptInput(e.target.value)}
            placeholder={t('settings.localModel.download.visionPromptPlaceholder')}
            rows={3}
            aria-label={t('settings.localModel.download.visionPromptPlaceholder')}
          />
          <SettingsTextArea
            value={visionImageInput}
            onChange={e => onSetVisionImageInput(e.target.value)}
            placeholder={t('settings.localModel.download.visionImagePlaceholder')}
            rows={3}
            aria-label={t('settings.localModel.download.visionImagePlaceholder')}
          />
          <Button
            type="button"
            variant="secondary"
            size="xs"
            onClick={onRunVisionTest}
            disabled={
              !runtimeEnabled ||
              isVisionLoading ||
              !visionPromptInput.trim() ||
              !visionImageInput.trim()
            }>
            {isVisionLoading
              ? t('settings.localModel.download.running')
              : t('settings.localModel.download.runVisionTest')}
          </Button>
          {visionOutput && (
            <pre className="whitespace-pre-wrap rounded-md bg-surface-muted border border-line p-3 text-xs text-content-secondary">
              {visionOutput}
            </pre>
          )}
        </div>
      </SettingsSection>

      <SettingsSection title={t('settings.localModel.download.testEmbeddings')}>
        <div className="px-4 py-3 space-y-3">
          <SettingsTextArea
            value={embeddingInput}
            onChange={e => onSetEmbeddingInput(e.target.value)}
            placeholder={t('settings.localModel.download.embeddingPlaceholder')}
            rows={3}
            aria-label={t('settings.localModel.download.embeddingPlaceholder')}
          />
          <Button
            type="button"
            variant="secondary"
            size="xs"
            onClick={onRunEmbeddingTest}
            disabled={!runtimeEnabled || isEmbeddingLoading || !embeddingInput.trim()}>
            {isEmbeddingLoading
              ? t('settings.localModel.download.running')
              : t('settings.localModel.download.runEmbeddingTest')}
          </Button>
          {embeddingOutput && (
            <div className="rounded-md bg-surface-muted border border-line p-3 text-xs text-content-secondary space-y-1">
              <div>
                {t('settings.localModel.download.embeddingModel').replace(
                  '{modelId}',
                  embeddingOutput.model_id
                )}
              </div>
              <div>
                {t('settings.localModel.download.embeddingDimensions').replace(
                  '{dimensions}',
                  String(embeddingOutput.dimensions)
                )}
              </div>
              <div>
                {t('settings.localModel.download.embeddingVectors').replace(
                  '{count}',
                  String(embeddingOutput.vectors.length)
                )}
              </div>
            </div>
          )}
        </div>
      </SettingsSection>

      <SettingsSection title={t('settings.localModel.download.testVoiceInput')}>
        <div className="px-4 py-3 space-y-3">
          <SettingsTextField
            value={audioPathInput}
            onChange={e => onSetAudioPathInput(e.target.value)}
            placeholder={t('settings.localModel.download.audioPathPlaceholder')}
            aria-label={t('settings.localModel.download.audioPathPlaceholder')}
          />
          <Button
            type="button"
            variant="secondary"
            size="xs"
            onClick={onRunTranscribeTest}
            disabled={!runtimeEnabled || isTranscribeLoading || !audioPathInput.trim()}>
            {isTranscribeLoading
              ? t('settings.localModel.download.running')
              : t('settings.localModel.download.runTranscriptionTest')}
          </Button>
          {transcribeOutput && (
            <div className="rounded-md bg-surface-muted border border-line p-3 text-xs text-content-secondary space-y-2">
              <div>
                {t('settings.localModel.download.embeddingModel').replace(
                  '{modelId}',
                  transcribeOutput.model_id
                )}
              </div>
              <div>
                <span className="text-content-muted">
                  {t('settings.localModel.download.transcript')}
                </span>
                <pre className="whitespace-pre-wrap mt-1">{transcribeOutput.text}</pre>
              </div>
            </div>
          )}
        </div>
      </SettingsSection>

      <SettingsSection title={t('settings.localModel.download.testVoiceOutput')}>
        <div className="px-4 py-3 space-y-3">
          <SettingsTextArea
            value={ttsInput}
            onChange={e => onSetTtsInput(e.target.value)}
            placeholder={t('settings.localModel.download.ttsPlaceholder')}
            rows={3}
            aria-label={t('settings.localModel.download.ttsPlaceholder')}
          />
          <SettingsTextField
            value={ttsOutputPath}
            onChange={e => onSetTtsOutputPath(e.target.value)}
            placeholder={t('settings.localModel.download.ttsOutputPlaceholder')}
            aria-label={t('settings.localModel.download.ttsOutputPlaceholder')}
          />
          <Button
            type="button"
            variant="secondary"
            size="xs"
            onClick={onRunTtsTest}
            disabled={!runtimeEnabled || isTtsLoading || !ttsInput.trim()}>
            {isTtsLoading
              ? t('settings.localModel.download.running')
              : t('settings.localModel.download.runTtsTest')}
          </Button>
          {ttsOutput && (
            <div className="rounded-md bg-surface-muted border border-line p-3 text-xs text-content-secondary space-y-1">
              <div>
                {t('settings.localModel.download.ttsVoice').replace(
                  '{voiceId}',
                  ttsOutput.voice_id
                )}
              </div>
              <div className="break-all">
                {t('settings.localModel.download.ttsOutput').replace(
                  '{outputPath}',
                  ttsOutput.output_path
                )}
              </div>
            </div>
          )}
        </div>
      </SettingsSection>
    </>
  );
};

export default ModelDownloadSection;
