import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { testProviderModel } from '../../../../../services/api/aiSettingsApi';
import { renderWithProviders } from '../../../../../test/test-utils';
import { CustomRoutingDialog } from '../CustomRoutingDialog';

vi.mock('../../../../../services/api/aiSettingsApi', async importOriginal => {
  const actual = await importOriginal<typeof import('../../../../../services/api/aiSettingsApi')>();
  return {
    ...actual,
    testProviderModel: vi.fn().mockResolvedValue({ reply: 'ok' }),
    modelRegistryVision: vi.fn(() => false),
  };
});

const workload = {
  id: 'reasoning',
  labelKey: 'settings.ai.workload.reasoning',
  descriptionKey: 'settings.ai.workload.reasoning.description',
} as never;

const renderDialog = (initial: Parameters<typeof CustomRoutingDialog>[0]['initial']) =>
  renderWithProviders(
    <CustomRoutingDialog
      workload={workload}
      initial={initial}
      cloudProviders={[
        { slug: 'claude-code', label: 'Claude Code', endpoint: '', authStyle: 'none' } as never,
        { slug: 'openai', label: 'OpenAI', endpoint: '', authStyle: 'bearer' } as never,
      ]}
      localModels={[]}
      ollamaRunning={false}
      modelRegistry={[]}
      onClose={() => {}}
      onSubmit={() => {}}
    />
  );

describe('CustomRoutingDialog test button', () => {
  beforeEach(() => {
    vi.mocked(testProviderModel).mockClear();
  });

  // Regression: the test call used to fall back to `ollama:<model>` for every
  // non-cloud source, so testing a Claude Code route asked Ollama for a model it
  // has never heard of — and the failure was reported against claude-code.
  it('tests a claude-code route under the claude-code slug, not ollama', async () => {
    renderDialog({ kind: 'claude-code', model: 'claude-fable-5-1', temperature: null } as never);

    fireEvent.click(screen.getByRole('button', { name: /test/i }));

    await waitFor(() => expect(testProviderModel).toHaveBeenCalled());
    const [, providerString] = vi.mocked(testProviderModel).mock.calls[0];
    expect(providerString).toBe('claude-code:claude-fable-5-1');
  });

  it('keeps naming the cloud provider slug for a cloud route', async () => {
    renderDialog({
      kind: 'cloud',
      providerSlug: 'openai',
      model: 'gpt-5.5',
      temperature: null,
    } as never);

    fireEvent.click(screen.getByRole('button', { name: /test/i }));

    await waitFor(() => expect(testProviderModel).toHaveBeenCalled());
    const [, providerString] = vi.mocked(testProviderModel).mock.calls[0];
    expect(providerString).toBe('openai:gpt-5.5');
  });
});
