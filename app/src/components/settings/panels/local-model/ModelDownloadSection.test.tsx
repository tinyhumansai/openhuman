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
