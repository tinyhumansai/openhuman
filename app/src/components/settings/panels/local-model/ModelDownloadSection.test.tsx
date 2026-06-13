import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import ModelDownloadSection from './ModelDownloadSection';

const makeProps = () => ({
  assets: null,
  assetDownloadBusy: {},
  statusTone: (_state: string) => '',
  runtimeEnabled: false,
  onTriggerAssetDownload: vi.fn(),
  summaryInput: 'summarize me',
  summaryOutput: '',
  isSummaryLoading: false,
  onSetSummaryInput: vi.fn(),
  onRunSummaryTest: vi.fn(),
  promptInput: 'prompt',
  promptOutput: '',
  promptError: '',
  isPromptLoading: false,
  promptNoThink: true,
  onSetPromptInput: vi.fn(),
  onSetPromptNoThink: vi.fn(),
  onRunPromptTest: vi.fn(),
  visionPromptInput: 'what is this?',
  visionImageInput: 'image-ref',
  visionOutput: '',
  isVisionLoading: false,
  onSetVisionPromptInput: vi.fn(),
  onSetVisionImageInput: vi.fn(),
  onRunVisionTest: vi.fn(),
  embeddingInput: 'one line',
  embeddingOutput: null,
  isEmbeddingLoading: false,
  onSetEmbeddingInput: vi.fn(),
  onRunEmbeddingTest: vi.fn(),
  audioPathInput: '/tmp/audio.wav',
  transcribeOutput: null,
  isTranscribeLoading: false,
  onSetAudioPathInput: vi.fn(),
  onRunTranscribeTest: vi.fn(),
  ttsInput: 'say this',
  ttsOutputPath: '',
  ttsOutput: null,
  isTtsLoading: false,
  onSetTtsInput: vi.fn(),
  onSetTtsOutputPath: vi.fn(),
  onRunTtsTest: vi.fn(),
});

// No i18n mock: existing tests assert on real English strings (e.g. 'Run Summary Test').
// New tests must also use real English translation strings.

describe('ModelDownloadSection runtime gate', () => {
  it('does not invoke local-AI test actions when runtime is disabled', () => {
    const props = makeProps();
    render(<ModelDownloadSection {...props} />);

    const summaryButton = screen.getByRole('button', { name: 'Run Summary Test' });
    expect(summaryButton).toBeDisabled();
    fireEvent.click(summaryButton);

    const promptButton = screen.getByRole('button', { name: 'Run Prompt Test' });
    expect(promptButton).toBeDisabled();
    fireEvent.click(promptButton);

    expect(props.onRunSummaryTest).not.toHaveBeenCalled();
    expect(props.onRunPromptTest).not.toHaveBeenCalled();
  });

  it('shows external-runtime guidance for ollama-backed assets', () => {
    render(
      <ModelDownloadSection
        {...makeProps()}
        runtimeEnabled={true}
        assets={{
          quantization: 'q4',
          chat: {
            id: 'gemma3:1b-it-qat',
            provider: 'ollama',
            state: 'missing',
            path: 'ollama://gemma3:1b-it-qat',
            warning: null,
          },
          vision: { id: '', provider: 'ollama', state: 'disabled', path: null, warning: null },
          embedding: {
            id: 'bge-m3',
            provider: 'ollama',
            state: 'missing',
            path: 'ollama://bge-m3',
            warning: null,
          },
          stt: { id: 'whisper', provider: 'whisper', state: 'ondemand', path: null, warning: null },
          tts: { id: 'piper', provider: 'piper', state: 'ondemand', path: null, warning: null },
          ollama_available: true,
        }}
      />
    );

    expect(
      screen.getAllByText('Manage this model in your external runtime.').length
    ).toBeGreaterThan(0);
    expect(screen.getAllByRole('button', { name: 'Download' }).length).toBeGreaterThan(0);
  });
});

describe('ModelDownloadSection — promptError, outputs (line 238)', () => {
  const baseProps = () => ({
    assets: null,
    assetDownloadBusy: {},
    statusTone: (_state: string) => '',
    runtimeEnabled: true,
    onTriggerAssetDownload: vi.fn(),
    summaryInput: '',
    summaryOutput: '',
    isSummaryLoading: false,
    onSetSummaryInput: vi.fn(),
    onRunSummaryTest: vi.fn(),
    promptInput: 'test prompt',
    promptOutput: '',
    promptError: '',
    isPromptLoading: false,
    promptNoThink: true,
    onSetPromptInput: vi.fn(),
    onSetPromptNoThink: vi.fn(),
    onRunPromptTest: vi.fn(),
    visionPromptInput: '',
    visionImageInput: '',
    visionOutput: '',
    isVisionLoading: false,
    onSetVisionPromptInput: vi.fn(),
    onSetVisionImageInput: vi.fn(),
    onRunVisionTest: vi.fn(),
    embeddingInput: '',
    embeddingOutput: null,
    isEmbeddingLoading: false,
    onSetEmbeddingInput: vi.fn(),
    onRunEmbeddingTest: vi.fn(),
    audioPathInput: '',
    transcribeOutput: null,
    isTranscribeLoading: false,
    onSetAudioPathInput: vi.fn(),
    onRunTranscribeTest: vi.fn(),
    ttsInput: '',
    ttsOutputPath: '',
    ttsOutput: null,
    isTtsLoading: false,
    onSetTtsInput: vi.fn(),
    onSetTtsOutputPath: vi.fn(),
    onRunTtsTest: vi.fn(),
  });

  it('renders promptError status line when promptError is non-empty (line 238)', () => {
    render(
      <ModelDownloadSection
        {...baseProps()}
        promptError="model returned an error: context exceeded"
      />
    );
    expect(screen.getByText('model returned an error: context exceeded')).toBeTruthy();
  });

  it('renders promptOutput pre when promptOutput is non-empty (line 239-243)', () => {
    render(
      <ModelDownloadSection {...baseProps()} promptOutput="The capital of France is Paris." />
    );
    expect(screen.getByText('The capital of France is Paris.')).toBeTruthy();
  });

  it('renders summaryOutput pre when summaryOutput is non-empty', () => {
    render(
      <ModelDownloadSection
        {...baseProps()}
        summaryInput="some text to summarize"
        summaryOutput="Summary: concise result"
      />
    );
    expect(screen.getByText('Summary: concise result')).toBeTruthy();
  });

  it('renders embeddingOutput details when non-null', () => {
    render(
      <ModelDownloadSection
        {...baseProps()}
        embeddingInput="word1\nword2"
        embeddingOutput={{ model_id: 'nomic-embed', dimensions: 768, vectors: [[0.1, 0.2]] }}
      />
    );
    expect(screen.getByText(/nomic-embed/)).toBeTruthy();
    expect(screen.getByText(/768/)).toBeTruthy();
  });

  it('renders transcribeOutput when non-null', () => {
    render(
      <ModelDownloadSection
        {...baseProps()}
        audioPathInput="/tmp/test.wav"
        transcribeOutput={{ model_id: 'whisper-tiny', text: 'hello world' }}
      />
    );
    expect(screen.getByText('hello world')).toBeTruthy();
  });

  it('renders ttsOutput when non-null', () => {
    render(
      <ModelDownloadSection
        {...baseProps()}
        ttsInput="speak this"
        ttsOutput={{ voice_id: 'en_US', output_path: '/tmp/output.wav' }}
      />
    );
    expect(screen.getByText(/en_US/)).toBeTruthy();
    expect(screen.getByText(/\/tmp\/output\.wav/)).toBeTruthy();
  });

  it('shows running prompt spinner when isPromptLoading (line 232-236)', () => {
    render(<ModelDownloadSection {...baseProps()} isPromptLoading={true} />);
    // Translation: 'settings.localModel.download.runningPrompt' → 'Running prompt'
    expect(screen.getByText('Running prompt')).toBeTruthy();
  });
});
