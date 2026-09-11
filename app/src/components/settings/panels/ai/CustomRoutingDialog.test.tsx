import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { serializeProviderRef, testProviderModel } from '../../../../services/api/aiSettingsApi';
import type { CloudProvider, ProviderRef, Workload } from './aiPanelTypes';
import { CustomRoutingDialog } from './CustomRoutingDialog';

// `serializeProviderRef` is deliberately NOT mocked: the temperature-parity test
// below asserts that Test serialises the same way Save does, and a mocked
// serialiser would make that assertion a tautology.
vi.mock('../../../../services/api/aiSettingsApi', async importOriginal => {
  const actual = await importOriginal<typeof import('../../../../services/api/aiSettingsApi')>();
  return {
    serializeProviderRef: actual.serializeProviderRef,
    testProviderModel: vi.fn(),
    describeProviderVerificationFailure: vi.fn(() => 'test failed'),
    modelRegistryVision: vi.fn(() => false),
    // `ProviderModelPickerDialog` imports this at module scope even though this
    // suite never opens the picker.
    listProviderModels: vi.fn(),
  };
});

const CHAT_WORKLOAD: Workload = {
  id: 'chat',
  group: 'chat',
  labelKey: 'settings.ai.routing.workload.chat.label',
  descriptionKey: 'settings.ai.routing.workload.chat.desc',
};

const CLAUDE_CODE_PROVIDER: CloudProvider = {
  id: 'claude-code',
  slug: 'claude-code',
  label: 'Claude Code',
  endpoint: '',
  authStyle: 'bearer',
  maskedKey: '',
};

const OPENAI_PROVIDER: CloudProvider = {
  id: 'openai',
  slug: 'openai',
  label: 'OpenAI',
  endpoint: 'https://api.openai.com/v1',
  authStyle: 'bearer',
  maskedKey: '••••',
};

/** Renders the dialog on `initial` and clicks Test, returning nothing — the
 *  assertions read `testProviderModel`'s recorded arguments. */
const renderAndTest = (initial: ProviderRef, cloudProviders: CloudProvider[]) => {
  render(
    <CustomRoutingDialog
      workload={CHAT_WORKLOAD}
      initial={initial}
      cloudProviders={cloudProviders}
      localModels={[]}
      ollamaRunning={false}
      modelRegistry={[]}
      onClose={() => {}}
      onSubmit={() => {}}
    />
  );
  fireEvent.click(screen.getByRole('button', { name: 'Test' }));
};

describe('CustomRoutingDialog — Test button provider string', () => {
  /**
   * #6125: `currentProviderString` branched only on `cloud`, so every other
   * non-managed kind fell into the `ollama:` arm. A Claude Code route was tested
   * against the local runtime — the CLI was never spawned and the test could
   * only fail, while the button above still read "Claude Code CLI · sonnet".
   */
  it('sends claude-code:<model> for a Claude Code CLI route', async () => {
    vi.mocked(testProviderModel).mockResolvedValue({ reply: 'hi' });

    renderAndTest({ kind: 'claude-code', model: 'sonnet' }, [CLAUDE_CODE_PROVIDER]);

    await waitFor(() =>
      expect(testProviderModel).toHaveBeenCalledWith('chat', 'claude-code:sonnet', 'Hello world')
    );
    // The banner renders the same string, which is how the two disagreed before:
    // the error text used `registrySlug` (correct) while the call used `ollama:`.
    expect(await screen.findByText('Provider: claude-code:sonnet')).toBeInTheDocument();
  });

  /**
   * Test must exercise exactly what Save persists. `TemperatureOverrideField` is
   * rendered for every source kind, so a claude-code route can carry an override
   * and `serializeProviderRef` writes it into the stored route.
   */
  it('appends the temperature override, matching what Save persists', async () => {
    vi.mocked(testProviderModel).mockResolvedValue({ reply: 'hi' });
    const saved: ProviderRef = { kind: 'claude-code', model: 'sonnet', temperature: 0.7 };

    renderAndTest(saved, [CLAUDE_CODE_PROVIDER]);

    await waitFor(() =>
      expect(testProviderModel).toHaveBeenCalledWith(
        'chat',
        'claude-code:sonnet@0.7',
        'Hello world'
      )
    );
    expect(vi.mocked(testProviderModel).mock.calls[0][1]).toBe(serializeProviderRef(saved));
  });

  it('leaves a cloud route unchanged', async () => {
    vi.mocked(testProviderModel).mockResolvedValue({ reply: 'hi' });

    renderAndTest({ kind: 'cloud', providerSlug: 'openai', model: 'gpt-4o' }, [OPENAI_PROVIDER]);

    await waitFor(() =>
      expect(testProviderModel).toHaveBeenCalledWith('chat', 'openai:gpt-4o', 'Hello world')
    );
  });

  it('leaves a local route on the ollama: prefix', async () => {
    vi.mocked(testProviderModel).mockResolvedValue({ reply: 'hi' });

    renderAndTest({ kind: 'local', model: 'llama3' }, []);

    await waitFor(() =>
      expect(testProviderModel).toHaveBeenCalledWith('chat', 'ollama:llama3', 'Hello world')
    );
  });

  /** Managed resolves its route server-side at request time, so there is no
   *  provider string to send and Test is disabled rather than sending `cloud`. */
  it('disables Test for a managed route', () => {
    render(
      <CustomRoutingDialog
        workload={CHAT_WORKLOAD}
        initial={{ kind: 'default' }}
        cloudProviders={[]}
        localModels={[]}
        ollamaRunning={false}
        modelRegistry={[]}
        onClose={() => {}}
        onSubmit={() => {}}
      />
    );

    expect(screen.getByRole('button', { name: 'Test' })).toBeDisabled();
    expect(testProviderModel).not.toHaveBeenCalled();
  });
});
