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
  it('offers managed first and waits for a model to be picked', () => {
    render(
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

    // Preselected; there is no "automatic" row any more, so nothing can be
    // submitted until a catalog model is chosen.
    expect(screen.getByTestId('model-picker-managed-pane')).toBeInTheDocument();
    expect(screen.queryByRole('combobox', { name: 'Model' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Use this model' })).toBeDisabled();
  });

  /**
   * The managed backend's OpenRouter passthrough catalog is offered under
   * "OpenRouter — Managed by TinyHumans" so a specific model can be pinned while still
   * billing through managed credits.
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

    // The catalog is a list filling the pane, not a dropdown: display name
    // and charged price are surfaced on the row, not the bare slug.
    const row = await screen.findByTestId(
      'model-picker-managed-option-openrouter/deepseek/deepseek-v4-flash'
    );
    expect(row).toHaveTextContent('DeepSeek V4 Flash');
    expect(row).toHaveTextContent('per 1M');

    fireEvent.click(row);
    expect(row).toHaveAttribute('aria-selected', 'true');
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

    await screen.findByTestId('model-picker-managed-option-openrouter/a/b');

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
      expect(screen.getByTestId('model-picker-managed-option-openrouter/a/b')).toBeInTheDocument()
    );
    expect(vi.mocked(listProviderModels)).toHaveBeenCalledTimes(1);
  });

  it('the search box narrows models and finds providers from the same field', async () => {
    vi.mocked(listProviderModels).mockResolvedValue([
      { id: 'openrouter/a/alpha', owned_by: 'openrouter', display_name: 'Alpha' },
      { id: 'openrouter/b/beta', owned_by: 'openrouter', display_name: 'Beta' },
    ]);

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
        onSelect={() => {}}
      />
    );
    await screen.findByTestId('model-picker-managed-option-openrouter/a/alpha');

    fireEvent.change(screen.getByRole('textbox', { name: 'Search providers and models' }), {
      target: { value: 'bet' },
    });

    expect(screen.queryByTestId('model-picker-managed-option-openrouter/a/alpha')).toBeNull();
    expect(screen.getByTestId('model-picker-managed-option-openrouter/b/beta')).toBeInTheDocument();
    // The selected provider stays available, but unrelated providers are filtered.
    expect(screen.queryByText('OpenAI')).not.toBeInTheDocument();
    expect(screen.getByText('OpenRouter')).toBeInTheDocument();

    fireEvent.change(screen.getByRole('textbox', { name: 'Search providers and models' }), {
      target: { value: 'OpenAI' },
    });
    expect(screen.getByRole('button', { name: /OpenAI/ })).toBeInTheDocument();
    expect(screen.queryByTestId('model-picker-managed-option-openrouter/b/beta')).toBeNull();
  });

  it('lists the managed catalog alphabetically by display name', async () => {
    vi.mocked(listProviderModels).mockResolvedValue([
      { id: 'openrouter/z/zeta', owned_by: 'openrouter', display_name: 'Zeta' },
      { id: 'openrouter/a/no-name', owned_by: 'openrouter' },
      { id: 'openrouter/b/beta', owned_by: 'openrouter', display_name: 'beta' },
    ]);

    render(
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
    await screen.findByTestId('model-picker-managed-option-openrouter/z/zeta');

    const order = screen
      .getAllByRole('option')
      .map(option =>
        option.getAttribute('data-testid')?.replace('model-picker-managed-option-', '')
      );
    // Case-insensitive, and an unnamed entry sorts by its id.
    expect(order).toEqual(['openrouter/b/beta', 'openrouter/a/no-name', 'openrouter/z/zeta']);
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
