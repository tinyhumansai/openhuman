import { useT } from '../../../../lib/i18n/I18nContext';
import { statusLabel } from '../../../../utils/localAiHelpers';
import type {
  LocalAiAssetsStatus,
  LocalAiEmbeddingResult,
  LocalAiSpeechResult,
  LocalAiTtsResult,
} from '../../../../utils/tauriCommands';

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
      <section className="space-y-3">
        <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
          {t('settings.localModel.download.capabilityAssets')}
        </h3>
        <div className="bg-stone-50 dark:bg-neutral-800/60 rounded-lg border border-stone-200 dark:border-neutral-800 p-4 space-y-3">
          <div className="text-xs text-stone-500 dark:text-neutral-400">
            {t('settings.localModel.download.quantizationPref')} {assets?.quantization ?? 'q4'}
          </div>
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 text-sm">
            {capabilityCards.map(([labelKey, key, item]) => (
              <div
                key={key}
                className="rounded-md border border-stone-200 dark:border-neutral-800 p-2">
                <div className="text-stone-500 dark:text-neutral-400 text-xs uppercase tracking-wide">
                  {t(labelKey)}
                </div>
                <div className="text-stone-800 dark:text-neutral-100 mt-1 break-all">
                  {item?.id ?? t('settings.localModel.download.notAvailable')}
                </div>
                <div className={`text-xs mt-1 ${statusTone(item?.state ?? 'idle')}`}>
                  {statusLabel(item?.state ?? 'idle')}
                </div>
                {item?.path && (
                  <div className="text-[10px] text-stone-500 dark:text-neutral-400 mt-1 break-all">
                    {item.path}
                  </div>
                )}
                {item?.provider === 'ollama' || item?.provider === 'lm_studio' ? (
                  <div className="mt-2 text-[10px] text-stone-500 dark:text-neutral-400">
                    {t('settings.localModel.download.manageExternal')}
                  </div>
                ) : (
                  <button
                    onClick={() => onTriggerAssetDownload(key)}
                    disabled={!runtimeEnabled || assetDownloadBusy[key]}
                    className="mt-2 px-2 py-1 text-[10px] rounded border border-stone-200 dark:border-neutral-800 hover:border-stone-300 dark:border-neutral-700 disabled:opacity-60 text-stone-600 dark:text-neutral-300">
                    {assetDownloadBusy[key]
                      ? t('settings.localModel.download.downloading')
                      : t('common.download')}
                  </button>
                )}
              </div>
            ))}
          </div>
        </div>
      </section>

      <section className="space-y-3">
        <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
          {t('settings.localModel.download.testSummarization')}
        </h3>
        <div className="bg-stone-50 dark:bg-neutral-800/60 rounded-lg border border-stone-200 dark:border-neutral-800 p-4 space-y-3">
          <textarea
            value={summaryInput}
            onChange={e => onSetSummaryInput(e.target.value)}
            placeholder={t('settings.localModel.download.summarizePlaceholder')}
            className="w-full min-h-28 rounded-md bg-white dark:bg-neutral-900 border border-stone-200 dark:border-neutral-800 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 dark:text-neutral-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
          />
          <div className="flex items-center justify-between">
            <div className="text-xs text-stone-500 dark:text-neutral-400">
              {t('settings.localModel.download.summaryHelper')}
            </div>
            <button
              onClick={onRunSummaryTest}
              disabled={!runtimeEnabled || isSummaryLoading || !summaryInput.trim()}
              className="px-3 py-1.5 text-xs rounded-md bg-emerald-600 hover:bg-emerald-700 disabled:opacity-60 text-white">
              {isSummaryLoading
                ? t('settings.localModel.download.running')
                : t('settings.localModel.download.runSummaryTest')}
            </button>
          </div>
          {summaryOutput && (
            <pre className="whitespace-pre-wrap rounded-md bg-stone-50 dark:bg-neutral-800/60 border border-stone-200 dark:border-neutral-800 p-3 text-xs text-stone-700 dark:text-neutral-200">
              {summaryOutput}
            </pre>
          )}
        </div>
      </section>

      <section className="space-y-3">
        <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
          {t('settings.localModel.download.testCustomPrompt')}
        </h3>
        <div className="bg-stone-50 dark:bg-neutral-800/60 rounded-lg border border-stone-200 dark:border-neutral-800 p-4 space-y-3">
          <textarea
            value={promptInput}
            onChange={e => onSetPromptInput(e.target.value)}
            placeholder={t('settings.localModel.download.promptPlaceholder')}
            className="w-full min-h-28 rounded-md bg-white dark:bg-neutral-900 border border-stone-200 dark:border-neutral-800 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 dark:text-neutral-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
          />
          <div className="flex flex-wrap items-center justify-between gap-2">
            <label className="flex items-center gap-2 text-xs text-stone-700 dark:text-neutral-200">
              <input
                type="checkbox"
                checked={promptNoThink}
                onChange={e => onSetPromptNoThink(e.target.checked)}
                className="h-3.5 w-3.5 rounded border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 text-primary-500 focus:ring-primary-500"
              />
              {t('settings.localModel.download.noThinkMode')}
            </label>
            <button
              onClick={onRunPromptTest}
              disabled={!runtimeEnabled || isPromptLoading || !promptInput.trim()}
              className="px-3 py-1.5 text-xs rounded-md bg-primary-600 hover:bg-primary-700 disabled:opacity-60 text-white">
              {isPromptLoading
                ? t('settings.localModel.download.running')
                : t('settings.localModel.download.runPromptTest')}
            </button>
          </div>
          {isPromptLoading && (
            <div className="flex items-center gap-2 text-xs text-primary-600 dark:text-primary-300">
              <div className="h-3 w-3 rounded-full border-2 border-blue-400 border-t-transparent animate-spin" />
              {t('settings.localModel.download.runningPrompt')}
            </div>
          )}
          {promptError && (
            <div className="rounded-md bg-red-50 dark:bg-red-500/10 border border-red-300 dark:border-red-500/40 p-3 text-xs text-red-600 dark:text-red-300">
              {promptError}
            </div>
          )}
          {promptOutput && (
            <pre className="whitespace-pre-wrap rounded-md bg-stone-50 dark:bg-neutral-800/60 border border-stone-200 dark:border-neutral-800 p-3 text-xs text-stone-700 dark:text-neutral-200 max-h-64 overflow-auto">
              {promptOutput}
            </pre>
          )}
        </div>
      </section>

      <section className="space-y-3">
        <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
          {t('settings.localModel.download.testVisionPrompt')}
        </h3>
        <div className="bg-stone-50 dark:bg-neutral-800/60 rounded-lg border border-stone-200 dark:border-neutral-800 p-4 space-y-3">
          <textarea
            value={visionPromptInput}
            onChange={e => onSetVisionPromptInput(e.target.value)}
            placeholder={t('settings.localModel.download.visionPromptPlaceholder')}
            className="w-full min-h-20 rounded-md bg-white dark:bg-neutral-900 border border-stone-200 dark:border-neutral-800 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 dark:text-neutral-500 focus:outline-none focus:ring-1 focus:ring-primary-400"
          />
          <textarea
            value={visionImageInput}
            onChange={e => onSetVisionImageInput(e.target.value)}
            placeholder={t('settings.localModel.download.visionImagePlaceholder')}
            className="w-full min-h-20 rounded-md bg-white dark:bg-neutral-900 border border-stone-200 dark:border-neutral-800 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 dark:text-neutral-500 focus:outline-none focus:ring-1 focus:ring-primary-400"
          />
          <button
            onClick={onRunVisionTest}
            disabled={
              !runtimeEnabled ||
              isVisionLoading ||
              !visionPromptInput.trim() ||
              !visionImageInput.trim()
            }
            className="px-3 py-1.5 text-xs rounded-md bg-indigo-600 hover:bg-indigo-700 disabled:opacity-60 text-white">
            {isVisionLoading
              ? t('settings.localModel.download.running')
              : t('settings.localModel.download.runVisionTest')}
          </button>
          {visionOutput && (
            <pre className="whitespace-pre-wrap rounded-md bg-stone-50 dark:bg-neutral-800/60 border border-stone-200 dark:border-neutral-800 p-3 text-xs text-stone-700 dark:text-neutral-200">
              {visionOutput}
            </pre>
          )}
        </div>
      </section>

      <section className="space-y-3">
        <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
          {t('settings.localModel.download.testEmbeddings')}
        </h3>
        <div className="bg-stone-50 dark:bg-neutral-800/60 rounded-lg border border-stone-200 dark:border-neutral-800 p-4 space-y-3">
          <textarea
            value={embeddingInput}
            onChange={e => onSetEmbeddingInput(e.target.value)}
            placeholder={t('settings.localModel.download.embeddingPlaceholder')}
            className="w-full min-h-20 rounded-md bg-white dark:bg-neutral-900 border border-stone-200 dark:border-neutral-800 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 dark:text-neutral-500 focus:outline-none focus:ring-1 focus:ring-primary-400"
          />
          <button
            onClick={onRunEmbeddingTest}
            disabled={!runtimeEnabled || isEmbeddingLoading || !embeddingInput.trim()}
            className="px-3 py-1.5 text-xs rounded-md bg-teal-600 hover:bg-teal-700 disabled:opacity-60 text-white">
            {isEmbeddingLoading
              ? t('settings.localModel.download.running')
              : t('settings.localModel.download.runEmbeddingTest')}
          </button>
          {embeddingOutput && (
            <div className="rounded-md bg-stone-50 dark:bg-neutral-800/60 border border-stone-200 dark:border-neutral-800 p-3 text-xs text-stone-700 dark:text-neutral-200 space-y-1">
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
      </section>

      <section className="space-y-3">
        <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
          {t('settings.localModel.download.testVoiceInput')}
        </h3>
        <div className="bg-stone-50 dark:bg-neutral-800/60 rounded-lg border border-stone-200 dark:border-neutral-800 p-4 space-y-3">
          <input
            value={audioPathInput}
            onChange={e => onSetAudioPathInput(e.target.value)}
            placeholder={t('settings.localModel.download.audioPathPlaceholder')}
            className="w-full rounded-md bg-white dark:bg-neutral-900 border border-stone-200 dark:border-neutral-800 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 dark:text-neutral-500 focus:outline-none focus:ring-1 focus:ring-primary-400"
          />
          <button
            onClick={onRunTranscribeTest}
            disabled={!runtimeEnabled || isTranscribeLoading || !audioPathInput.trim()}
            className="px-3 py-1.5 text-xs rounded-md bg-purple-600 hover:bg-purple-700 disabled:opacity-60 text-white">
            {isTranscribeLoading
              ? t('settings.localModel.download.running')
              : t('settings.localModel.download.runTranscriptionTest')}
          </button>
          {transcribeOutput && (
            <div className="rounded-md bg-stone-50 dark:bg-neutral-800/60 border border-stone-200 dark:border-neutral-800 p-3 text-xs text-stone-700 dark:text-neutral-200 space-y-2">
              <div>
                {t('settings.localModel.download.embeddingModel').replace(
                  '{modelId}',
                  transcribeOutput.model_id
                )}
              </div>
              <div>
                <span className="text-stone-400 dark:text-neutral-500">
                  {t('settings.localModel.download.transcript')}
                </span>
                <pre className="whitespace-pre-wrap mt-1">{transcribeOutput.text}</pre>
              </div>
            </div>
          )}
        </div>
      </section>

      <section className="space-y-3">
        <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
          {t('settings.localModel.download.testVoiceOutput')}
        </h3>
        <div className="bg-stone-50 dark:bg-neutral-800/60 rounded-lg border border-stone-200 dark:border-neutral-800 p-4 space-y-3">
          <textarea
            value={ttsInput}
            onChange={e => onSetTtsInput(e.target.value)}
            placeholder={t('settings.localModel.download.ttsPlaceholder')}
            className="w-full min-h-20 rounded-md bg-white dark:bg-neutral-900 border border-stone-200 dark:border-neutral-800 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 dark:text-neutral-500 focus:outline-none focus:ring-1 focus:ring-primary-400"
          />
          <input
            value={ttsOutputPath}
            onChange={e => onSetTtsOutputPath(e.target.value)}
            placeholder={t('settings.localModel.download.ttsOutputPlaceholder')}
            className="w-full rounded-md bg-white dark:bg-neutral-900 border border-stone-200 dark:border-neutral-800 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 dark:text-neutral-500 focus:outline-none focus:ring-1 focus:ring-primary-400"
          />
          <button
            onClick={onRunTtsTest}
            disabled={!runtimeEnabled || isTtsLoading || !ttsInput.trim()}
            className="px-3 py-1.5 text-xs rounded-md bg-rose-600 hover:bg-rose-700 disabled:opacity-60 text-white">
            {isTtsLoading
              ? t('settings.localModel.download.running')
              : t('settings.localModel.download.runTtsTest')}
          </button>
          {ttsOutput && (
            <div className="rounded-md bg-stone-50 dark:bg-neutral-800/60 border border-stone-200 dark:border-neutral-800 p-3 text-xs text-stone-700 dark:text-neutral-200 space-y-1">
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
      </section>
    </>
  );
};

export default ModelDownloadSection;
