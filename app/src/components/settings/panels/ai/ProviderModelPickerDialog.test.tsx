import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { listProviderModels } from '../../../../services/api/aiSettingsApi';
import { ProviderModelPickerDialog } from './ProviderModelPickerDialog';

vi.mock('../../../../services/api/aiSettingsApi', () => ({ listProviderModels: vi.fn() }));

describe('ProviderModelPickerDialog', () => {
  it('returns the provider-reported context window with a catalog selection', async () => {
    vi.mocked(listProviderModels).mockResolvedValue([
      { id: 'gpt-4o-mini', owned_by: 'openai', context_window: 128_000 },
    ]);
    const onSelect = vi.fn();

    render(
      <ProviderModelPickerDialog
        cloudProviders={[
          {
            id: 'openai',
            slug: 'openai',
            label: 'OpenAI',
            endpoint: 'https://api.openai.com/v1',
            authStyle: 'bearer',
            maskedKey: '••••',
          },
        ]}
        localModels={[]}
        ollamaRunning={false}
        claudeCodeEnabled={false}
        initial={null}
        onClose={() => {}}
        onSelect={onSelect}
      />
    );

    // Managed is the first source now, so reaching a provider's catalog means
    // selecting that provider — the same step a user takes.
    fireEvent.click(screen.getByRole('button', { name: /OpenAI/ }));

    fireEvent.change(await screen.findByRole('combobox', { name: 'Model' }), {
      target: { value: 'gpt-4o-mini' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Use this model' }));

    await waitFor(() =>
      expect(onSelect).toHaveBeenCalledWith({
        source: { kind: 'cloud', providerSlug: 'openai' },
        model: 'gpt-4o-mini',
        contextWindow: 128_000,
      })
    );
  });

  /**
   * Managed must be reachable from every model picker. Without it, choosing any
   * specific model was a one-way door: nothing in the UI routed back to the
   * product's own model selection.
   */
  it('offers managed first and selects it without requiring a model id', async () => {
    const onSelect = vi.fn();

    render(
      <ProviderModelPickerDialog
        cloudProviders={[]}
        localModels={[]}
        ollamaRunning={false}
        claudeCodeEnabled={false}
        initial={null}
        onClose={() => {}}
        onSelect={onSelect}
      />
    );

    // Preselected, and its pane explains the choice rather than asking for one.
    expect(screen.getByTestId('model-picker-managed-pane')).toBeInTheDocument();
    expect(screen.queryByRole('combobox', { name: 'Model' })).toBeNull();

    const submit = screen.getByRole('button', { name: 'Use this model' });
    expect(submit).not.toBeDisabled();
    fireEvent.click(submit);

    await waitFor(() =>
      expect(onSelect).toHaveBeenCalledWith({
        source: { kind: 'managed' },
        model: '',
        contextWindow: null,
      })
    );
  });

  /**
   * The managed backend's OpenRouter passthrough catalog is offered under
   * "Managed by OpenHuman" so a specific model can be pinned while still
   * billing through managed credits. Automatic stays the default.
   */
  it('lists the managed catalog and forwards a pinned model', async () => {
    vi.mocked(listProviderModels).mockResolvedValue([
      {
        id: 'openrouter/deepseek/deepseek-v4-flash',
        owned_by: 'openrouter',
        context_window: 1_000_000,
        display_name: 'DeepSeek V4 Flash',
        input_per_1m: 0.0886,
        output_per_1m: 0.1772,
      },
    ]);
    const onSelect = vi.fn();

    render(
      <ProviderModelPickerDialog
        cloudProviders={[]}
        localModels={[]}
        ollamaRunning={false}
        claudeCodeEnabled={false}
        initial={null}
        onClose={() => {}}
        onSelect={onSelect}
      />
    );

    // Fetched with the managed slug, not a BYOK provider id.
    await waitFor(() => expect(listProviderModels).toHaveBeenCalledWith('openhuman'));

    const select = await screen.findByTestId('model-picker-managed-select');
    // Display name and charged price are surfaced, not the bare slug.
    expect(select).toHaveTextContent('DeepSeek V4 Flash');
    expect(select).toHaveTextContent('per 1M');

    fireEvent.change(select, { target: { value: 'openrouter/deepseek/deepseek-v4-flash' } });
    fireEvent.click(screen.getByRole('button', { name: 'Use this model' }));

    await waitFor(() =>
      expect(onSelect).toHaveBeenCalledWith({
        source: { kind: 'managed' },
        model: 'openrouter/deepseek/deepseek-v4-flash',
        contextWindow: 1_000_000,
      })
    );
  });

  /**
   * Pinning must be reversible: an "Automatic" option is always present, unlike
   * ModelEntryField's empty option which disappears once a model is chosen.
   */
  it('can clear a pinned managed model back to automatic', async () => {
    vi.mocked(listProviderModels).mockResolvedValue([
      { id: 'openrouter/a/b', owned_by: 'openrouter', context_window: 1000 },
    ]);
    const onSelect = vi.fn();

    render(
      <ProviderModelPickerDialog
        cloudProviders={[]}
        localModels={[]}
        ollamaRunning={false}
        claudeCodeEnabled={false}
        initial={{ source: { kind: 'managed' }, model: 'openrouter/a/b' }}
        onClose={() => {}}
        onSelect={onSelect}
      />
    );

    const select = await screen.findByTestId('model-picker-managed-select');
    fireEvent.change(select, { target: { value: '' } });
    fireEvent.click(screen.getByRole('button', { name: 'Use this model' }));

    await waitFor(() =>
      expect(onSelect).toHaveBeenCalledWith({
        source: { kind: 'managed' },
        model: '',
        contextWindow: null,
      })
    );
  });

  /**
   * Regression: the fetch effect depended on the `source` object and the
   * `localModels` array. Callers pass those inline (`localModels={[]}`), so
   * their identity changed every render and the effect re-ran each time —
   * one network fetch per render, with the dropdown visibly thrashing.
   * The effect now keys off a derived slug string.
   */
  it('fetches the managed catalog once, not once per render', async () => {
    vi.mocked(listProviderModels).mockResolvedValue([
      { id: 'openrouter/a/b', owned_by: 'openrouter', context_window: 1000 },
    ]);

    const { rerender } = render(
      <ProviderModelPickerDialog
        cloudProviders={[]}
        localModels={[]}
        ollamaRunning={false}
        claudeCodeEnabled={false}
        initial={null}
        onClose={() => {}}
        onSelect={() => {}}
      />
    );

    await screen.findByTestId('model-picker-managed-select');

    // Re-render with fresh inline props, exactly as the real callers do.
    for (let i = 0; i < 3; i += 1) {
      rerender(
        <ProviderModelPickerDialog
          cloudProviders={[]}
          localModels={[]}
          ollamaRunning={false}
          claudeCodeEnabled={false}
          initial={null}
          onClose={() => {}}
          onSelect={() => {}}
        />
      );
    }

    await waitFor(() =>
      expect(screen.getByTestId('model-picker-managed-select')).toBeInTheDocument()
    );
    expect(vi.mocked(listProviderModels)).toHaveBeenCalledTimes(1);
  });

  it('omits managed when the host opts out', () => {
    render(
      <ProviderModelPickerDialog
        allowManaged={false}
        cloudProviders={[]}
        localModels={[]}
        ollamaRunning={false}
        claudeCodeEnabled={false}
        initial={null}
        onClose={() => {}}
        onSelect={() => {}}
      />
    );

    expect(screen.queryByTestId('model-picker-managed-pane')).toBeNull();
  });
});
