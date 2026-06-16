/**
 * EmbeddingsPanel unit tests — covers the uncovered changed lines:
 * 353-355, 361, 363, 365, 374-376, 386, 430, 432-433, 451, 453-454,
 * 470, 489, 491, 596, 659, 701, 704
 *
 * Exercises: provider radio selection, setup popup (API key entry, test,
 * save), confirm-wipe dialog, clear-key flow, test-connection flow, and
 * model / dimensions selects.
 */
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  clearEmbeddingsApiKey,
  type EmbeddingProviderEntry,
  type EmbeddingsSettings,
  loadEmbeddingsSettings,
  setEmbeddingsApiKey,
  testEmbeddingsConnection,
  updateEmbeddingsSettings,
} from '../../../../services/api/embeddingsApi';
import { renderWithProviders } from '../../../../test/test-utils';
import EmbeddingsPanel from '../EmbeddingsPanel';

vi.mock('../../../../services/api/embeddingsApi', () => ({
  loadEmbeddingsSettings: vi.fn(),
  updateEmbeddingsSettings: vi.fn(),
  setEmbeddingsApiKey: vi.fn(),
  clearEmbeddingsApiKey: vi.fn(),
  testEmbeddingsConnection: vi.fn(),
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    navigateToSettings: vi.fn(),
    breadcrumbs: [],
  }),
}));

const makeProvider = (
  slug: string,
  overrides: Partial<EmbeddingProviderEntry> = {}
): EmbeddingProviderEntry => ({
  slug,
  label: slug.charAt(0).toUpperCase() + slug.slice(1),
  description: `${slug} embeddings provider`,
  requires_api_key: false,
  requires_endpoint: false,
  has_api_key: false,
  models: [
    {
      id: `${slug}-model-v1`,
      label: `${slug} Model v1`,
      default_dimensions: 1536,
      allowed_dimensions: [768, 1536],
    },
  ],
  ...overrides,
});

const makeSettings = (overrides: Partial<EmbeddingsSettings> = {}): EmbeddingsSettings => ({
  provider: 'managed',
  model: 'managed-model-v1',
  dimensions: 1536,
  rate_limit_per_min: 60,
  vector_search_enabled: true,
  providers: [
    makeProvider('managed', { requires_api_key: false }),
    makeProvider('openai', { requires_api_key: true }),
    makeProvider('custom', { requires_api_key: false, requires_endpoint: true }),
  ],
  ...overrides,
});

describe('EmbeddingsPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(loadEmbeddingsSettings).mockResolvedValue(makeSettings());
    vi.mocked(updateEmbeddingsSettings).mockResolvedValue({
      provider: 'managed',
      model: 'managed-model-v1',
      dimensions: 1536,
    });
    vi.mocked(setEmbeddingsApiKey).mockResolvedValue(undefined);
    vi.mocked(clearEmbeddingsApiKey).mockResolvedValue(undefined);
    vi.mocked(testEmbeddingsConnection).mockResolvedValue({
      success: true,
      provider: 'managed',
      model: 'managed-model-v1',
      actual_dimensions: 1536,
    });
  });

  // ─── Initial load ─────────────────────────────────────────────────────────

  it('loads and renders provider options', async () => {
    renderWithProviders(<EmbeddingsPanel />);
    expect(await screen.findByText('Managed')).toBeInTheDocument();
    expect(screen.getByText('Openai')).toBeInTheDocument();
    expect(screen.getByText('Custom')).toBeInTheDocument();
  });

  it('shows loading state then settings', async () => {
    let resolveLoad!: (s: EmbeddingsSettings) => void;
    vi.mocked(loadEmbeddingsSettings).mockReturnValue(
      new Promise(r => {
        resolveLoad = r;
      })
    );
    renderWithProviders(<EmbeddingsPanel />);
    // loading placeholder visible
    expect(screen.getByText(/loading/i)).toBeInTheDocument();
    resolveLoad(makeSettings());
    expect(await screen.findByText('Managed')).toBeInTheDocument();
  });

  it('shows error state when loadEmbeddingsSettings rejects', async () => {
    vi.mocked(loadEmbeddingsSettings).mockRejectedValueOnce(new Error('network error'));
    renderWithProviders(<EmbeddingsPanel />);
    expect(await screen.findByText('network error')).toBeInTheDocument();
  });

  // ─── Provider selection — no API key needed ───────────────────────────────

  it('clicking a provider that needs no API key calls updateEmbeddingsSettings', async () => {
    // Start with openai selected so managed is a valid switch target
    const settings = makeSettings({
      provider: 'openai',
      providers: [
        makeProvider('managed', { requires_api_key: false }),
        makeProvider('openai', { requires_api_key: true, has_api_key: true }),
      ],
    });
    vi.mocked(loadEmbeddingsSettings).mockResolvedValue(settings);

    renderWithProviders(<EmbeddingsPanel />);
    await screen.findByText('Managed');

    // Click managed (no API key required, different from current)
    const managedBtn = screen.getByRole('radio', { name: /managed/i });
    fireEvent.click(managedBtn);

    await waitFor(() =>
      expect(vi.mocked(updateEmbeddingsSettings)).toHaveBeenCalledWith(
        expect.objectContaining({ provider: 'managed', confirm_wipe: false })
      )
    );
  });

  it('clicking the already-selected provider is a no-op', async () => {
    renderWithProviders(<EmbeddingsPanel />);
    await screen.findByText('Managed');

    const managedBtn = screen.getByRole('radio', { name: /managed/i });
    fireEvent.click(managedBtn);

    // No RPC — already selected
    await new Promise(r => setTimeout(r, 50));
    expect(vi.mocked(updateEmbeddingsSettings)).not.toHaveBeenCalled();
  });

  // ─── Setup popup — API key entry ──────────────────────────────────────────

  it('clicking a provider that requires an API key opens the setup popup', async () => {
    const settings = makeSettings({
      providers: [
        makeProvider('managed', { requires_api_key: false }),
        makeProvider('openai', { requires_api_key: true, has_api_key: false }),
      ],
    });
    vi.mocked(loadEmbeddingsSettings).mockResolvedValue(settings);

    renderWithProviders(<EmbeddingsPanel />);
    await screen.findByText('Openai');

    fireEvent.click(screen.getByRole('radio', { name: /openai/i }));

    // Setup popup appears with "Set up Openai" heading
    await waitFor(() =>
      expect(screen.getByRole('heading', { name: /set up openai/i })).toBeInTheDocument()
    );
  });

  it('can enter an API key and click Save & Switch to persist and switch provider', async () => {
    const settings = makeSettings({
      providers: [
        makeProvider('managed', { requires_api_key: false }),
        makeProvider('openai', { requires_api_key: true, has_api_key: false }),
      ],
    });
    vi.mocked(loadEmbeddingsSettings).mockResolvedValue(settings);
    vi.mocked(updateEmbeddingsSettings).mockResolvedValue({ provider: 'openai' });

    renderWithProviders(<EmbeddingsPanel />);
    await screen.findByText('Openai');

    fireEvent.click(screen.getByRole('radio', { name: /openai/i }));
    await screen.findByRole('heading', { name: /set up openai/i });

    // Enter key
    const keyInput = screen.getByPlaceholderText(/paste your api key/i);
    fireEvent.change(keyInput, { target: { value: 'sk-test-openai-key-12345' } });

    // Click Save & Switch
    fireEvent.click(screen.getByRole('button', { name: /save.*switch/i }));

    await waitFor(() => expect(vi.mocked(setEmbeddingsApiKey)).toHaveBeenCalled());
    await waitFor(() =>
      expect(vi.mocked(updateEmbeddingsSettings)).toHaveBeenCalledWith(
        expect.objectContaining({ provider: 'openai' })
      )
    );
  });

  it('can click Test Connection inside the setup popup', async () => {
    const settings = makeSettings({
      providers: [
        makeProvider('managed', { requires_api_key: false }),
        makeProvider('openai', { requires_api_key: true, has_api_key: false }),
      ],
    });
    vi.mocked(loadEmbeddingsSettings).mockResolvedValue(settings);
    vi.mocked(testEmbeddingsConnection).mockResolvedValue({
      success: true,
      provider: 'openai',
      model: 'openai-model-v1',
      actual_dimensions: 1536,
    });

    renderWithProviders(<EmbeddingsPanel />);
    await screen.findByText('Openai');

    fireEvent.click(screen.getByRole('radio', { name: /openai/i }));
    await screen.findByRole('heading', { name: /set up openai/i });

    const keyInput = screen.getByPlaceholderText(/paste your api key/i);
    fireEvent.change(keyInput, { target: { value: 'sk-test-key-abcdefgh' } });

    // There are two "Test connection" buttons (popup + outside panel); click the
    // first one which belongs to the setup popup (popup renders at end of DOM
    // but buttons are ordered — take first match which is the popup one).
    const testBtns = screen.getAllByRole('button', { name: /test connection/i });
    // The popup's Test Connection is disabled when key is empty (but we filled it),
    // and the outside one is enabled too — click the popup footer btn which is last
    fireEvent.click(testBtns[testBtns.length - 1]);

    await waitFor(() => expect(vi.mocked(testEmbeddingsConnection)).toHaveBeenCalled());
  });

  it('the setup popup Cancel button closes the popup without persisting', async () => {
    const settings = makeSettings({
      providers: [
        makeProvider('managed', { requires_api_key: false }),
        makeProvider('openai', { requires_api_key: true, has_api_key: false }),
      ],
    });
    vi.mocked(loadEmbeddingsSettings).mockResolvedValue(settings);

    renderWithProviders(<EmbeddingsPanel />);
    await screen.findByText('Openai');

    fireEvent.click(screen.getByRole('radio', { name: /openai/i }));
    await screen.findByRole('heading', { name: /set up openai/i });

    fireEvent.click(screen.getByRole('button', { name: /^cancel$/i }));

    await waitFor(() =>
      expect(screen.queryByRole('heading', { name: /set up openai/i })).not.toBeInTheDocument()
    );
    expect(vi.mocked(updateEmbeddingsSettings)).not.toHaveBeenCalled();
  });

  it('the show/hide key toggle inside the popup toggles input type', async () => {
    const settings = makeSettings({
      providers: [
        makeProvider('managed', { requires_api_key: false }),
        makeProvider('openai', { requires_api_key: true, has_api_key: false }),
      ],
    });
    vi.mocked(loadEmbeddingsSettings).mockResolvedValue(settings);

    renderWithProviders(<EmbeddingsPanel />);
    await screen.findByText('Openai');

    fireEvent.click(screen.getByRole('radio', { name: /openai/i }));
    await screen.findByRole('heading', { name: /set up openai/i });

    const keyInput = screen.getByPlaceholderText(/paste your api key/i) as HTMLInputElement;
    expect(keyInput.type).toBe('password');

    // Toggle show
    fireEvent.click(screen.getByRole('button', { name: /show/i }));
    expect(keyInput.type).toBe('text');

    // Toggle hide
    fireEvent.click(screen.getByRole('button', { name: /hide/i }));
    expect(keyInput.type).toBe('password');
  });

  // ─── Custom provider popup ─────────────────────────────────────────────────

  it('clicking the custom provider opens the custom endpoint form', async () => {
    const settings = makeSettings({
      providers: [
        makeProvider('managed', { requires_api_key: false }),
        makeProvider('custom', { requires_api_key: false, requires_endpoint: true }),
      ],
    });
    vi.mocked(loadEmbeddingsSettings).mockResolvedValue(settings);

    renderWithProviders(<EmbeddingsPanel />);
    await screen.findByText('Custom');

    fireEvent.click(screen.getByRole('radio', { name: /custom/i }));

    // Custom popup has an endpoint input
    await waitFor(() =>
      expect(screen.getByPlaceholderText(/https:\/\/your-endpoint/i)).toBeInTheDocument()
    );
  });

  it('can fill and save the custom endpoint form', async () => {
    const settings = makeSettings({
      providers: [
        makeProvider('managed', { requires_api_key: false }),
        makeProvider('custom', { requires_api_key: false, requires_endpoint: true }),
      ],
    });
    vi.mocked(loadEmbeddingsSettings).mockResolvedValue(settings);
    vi.mocked(updateEmbeddingsSettings).mockResolvedValue({ provider: 'custom' });

    renderWithProviders(<EmbeddingsPanel />);
    await screen.findByText('Custom');

    fireEvent.click(screen.getByRole('radio', { name: /custom/i }));
    await screen.findByPlaceholderText(/https:\/\/your-endpoint/i);

    const endpointInput = screen.getByPlaceholderText(/https:\/\/your-endpoint/i);
    fireEvent.change(endpointInput, { target: { value: 'https://my-embeddings.example.com/v1' } });

    const saveBtn = screen.getByRole('button', { name: /save.*switch/i });
    fireEvent.click(saveBtn);

    await waitFor(() =>
      expect(vi.mocked(updateEmbeddingsSettings)).toHaveBeenCalledWith(
        expect.objectContaining({
          provider: 'custom',
          custom_endpoint: 'https://my-embeddings.example.com/v1',
        })
      )
    );
  });

  it('surfaces an actionable error and keeps the popup open when the custom endpoint has no embeddings API', async () => {
    // TAURI-RUST-5JR: the backend probes the endpoint and rejects a chat-only
    // URL (DeepSeek) with EMBEDDINGS_ENDPOINT_NO_API. The panel must show the
    // message and NOT close the setup popup, so the user can fix the endpoint.
    const settings = makeSettings({
      providers: [
        makeProvider('managed', { requires_api_key: false }),
        makeProvider('custom', { requires_api_key: false, requires_endpoint: true }),
      ],
    });
    vi.mocked(loadEmbeddingsSettings).mockResolvedValue(settings);
    vi.mocked(updateEmbeddingsSettings).mockResolvedValue({
      error: 'EMBEDDINGS_ENDPOINT_NO_API',
      message: 'This endpoint has no embeddings API. Choose an embeddings-capable provider.',
    });

    renderWithProviders(<EmbeddingsPanel />);
    await screen.findByText('Custom');

    fireEvent.click(screen.getByRole('radio', { name: /custom/i }));
    await screen.findByPlaceholderText(/https:\/\/your-endpoint/i);
    fireEvent.change(screen.getByPlaceholderText(/https:\/\/your-endpoint/i), {
      target: { value: 'https://api.deepseek.com/v1' },
    });
    fireEvent.click(screen.getByRole('button', { name: /save.*switch/i }));

    // Actionable message shown.
    await screen.findByText(/no embeddings API/i);
    // Popup stays open — endpoint input is still present so the user can fix it.
    expect(screen.getByPlaceholderText(/https:\/\/your-endpoint/i)).toBeInTheDocument();
  });

  // ─── Confirm wipe dialog ──────────────────────────────────────────────────

  it('shows confirm-wipe dialog when updateEmbeddingsSettings returns DIMENSION_CHANGE error', async () => {
    const settings = makeSettings({
      provider: 'openai',
      providers: [
        makeProvider('managed', { requires_api_key: false }),
        makeProvider('openai', { requires_api_key: true, has_api_key: true }),
      ],
    });
    vi.mocked(loadEmbeddingsSettings).mockResolvedValue(settings);
    vi.mocked(updateEmbeddingsSettings).mockResolvedValue({
      error: 'EMBEDDINGS_DIMENSION_CHANGE_REQUIRES_WIPE',
    });

    renderWithProviders(<EmbeddingsPanel />);
    await screen.findByText('Managed');

    fireEvent.click(screen.getByRole('radio', { name: /managed/i }));

    // Wipe confirmation dialog appears
    await waitFor(() =>
      expect(screen.getByRole('heading', { name: /reset memory vectors/i })).toBeInTheDocument()
    );
  });

  it('confirm wipe calls updateEmbeddingsSettings with confirm_wipe: true', async () => {
    const settings = makeSettings({
      provider: 'openai',
      providers: [
        makeProvider('managed', { requires_api_key: false }),
        makeProvider('openai', { requires_api_key: true, has_api_key: true }),
      ],
    });
    vi.mocked(loadEmbeddingsSettings).mockResolvedValue(settings);
    // First call: wipe required; second: success
    vi.mocked(updateEmbeddingsSettings)
      .mockResolvedValueOnce({ error: 'EMBEDDINGS_DIMENSION_CHANGE_REQUIRES_WIPE' })
      .mockResolvedValueOnce({ provider: 'managed' });

    renderWithProviders(<EmbeddingsPanel />);
    await screen.findByText('Managed');

    fireEvent.click(screen.getByRole('radio', { name: /managed/i }));
    await waitFor(() =>
      expect(screen.getByRole('heading', { name: /reset memory vectors/i })).toBeInTheDocument()
    );

    // Click the confirm/wipe button (i18n key: settings.embeddings.confirmWipe = 'Wipe & apply')
    const confirmBtn = screen.getByRole('button', { name: /wipe.*apply|wipe & apply/i });
    fireEvent.click(confirmBtn);

    await waitFor(() =>
      expect(vi.mocked(updateEmbeddingsSettings)).toHaveBeenCalledWith(
        expect.objectContaining({ confirm_wipe: true })
      )
    );
  });

  it('cancel wipe dialog closes it without a second RPC call', async () => {
    const settings = makeSettings({
      provider: 'openai',
      providers: [
        makeProvider('managed', { requires_api_key: false }),
        makeProvider('openai', { requires_api_key: true, has_api_key: true }),
      ],
    });
    vi.mocked(loadEmbeddingsSettings).mockResolvedValue(settings);
    vi.mocked(updateEmbeddingsSettings).mockResolvedValue({
      error: 'EMBEDDINGS_DIMENSION_CHANGE_REQUIRES_WIPE',
    });

    renderWithProviders(<EmbeddingsPanel />);
    await screen.findByText('Managed');

    fireEvent.click(screen.getByRole('radio', { name: /managed/i }));
    await screen.findByRole('heading', { name: /reset memory vectors/i });

    fireEvent.click(screen.getByRole('button', { name: /^cancel$/i }));

    await waitFor(() =>
      expect(
        screen.queryByRole('heading', { name: /reset memory vectors/i })
      ).not.toBeInTheDocument()
    );
    // Only 1 RPC call — the one that returned wipe-required
    expect(vi.mocked(updateEmbeddingsSettings)).toHaveBeenCalledTimes(1);
  });

  // ─── Clear API key button ─────────────────────────────────────────────────

  it('clicking Clear API key calls clearEmbeddingsApiKey and reloads', async () => {
    const settings = makeSettings({
      provider: 'openai',
      providers: [
        makeProvider('openai', {
          requires_api_key: true,
          has_api_key: true,
          models: [
            {
              id: 'openai-model-v1',
              label: 'OpenAI Model v1',
              default_dimensions: 1536,
              allowed_dimensions: [1536],
            },
          ],
        }),
      ],
    });
    vi.mocked(loadEmbeddingsSettings).mockResolvedValue(settings);

    renderWithProviders(<EmbeddingsPanel />);
    await screen.findByText('Openai');

    // The "Clear API key" button is rendered only when currentEntry has_api_key=true
    const clearBtn = await screen.findByRole('button', { name: /clear api key/i });
    fireEvent.click(clearBtn);

    await waitFor(() => expect(vi.mocked(clearEmbeddingsApiKey)).toHaveBeenCalledWith('openai'));
  });

  // ─── Test Connection button (outside popup) ───────────────────────────────

  it('clicking Test Connection (outside popup) calls testEmbeddingsConnection', async () => {
    const settings = makeSettings({
      provider: 'managed',
      providers: [
        makeProvider('managed', {
          requires_api_key: false,
          models: [
            {
              id: 'managed-model-v1',
              label: 'Managed Model v1',
              default_dimensions: 1536,
              allowed_dimensions: [1536],
            },
          ],
        }),
      ],
    });
    vi.mocked(loadEmbeddingsSettings).mockResolvedValue(settings);

    renderWithProviders(<EmbeddingsPanel />);
    await screen.findByText('Managed');

    const testBtn = await screen.findByRole('button', { name: /test connection/i });
    fireEvent.click(testBtn);

    await waitFor(() => expect(vi.mocked(testEmbeddingsConnection)).toHaveBeenCalled());
  });

  it('shows error when testEmbeddingsConnection returns failure', async () => {
    const settings = makeSettings({
      provider: 'managed',
      providers: [
        makeProvider('managed', {
          requires_api_key: false,
          models: [
            {
              id: 'managed-model-v1',
              label: 'Managed Model v1',
              default_dimensions: 1536,
              allowed_dimensions: [1536],
            },
          ],
        }),
      ],
    });
    vi.mocked(loadEmbeddingsSettings).mockResolvedValue(settings);
    vi.mocked(testEmbeddingsConnection).mockResolvedValueOnce({
      success: false,
      provider: 'managed',
      model: 'managed-model-v1',
      error: 'Connection refused',
    });

    renderWithProviders(<EmbeddingsPanel />);
    await screen.findByText('Managed');

    const testBtn = await screen.findByRole('button', { name: /test connection/i });
    fireEvent.click(testBtn);

    await waitFor(() => expect(screen.getByText(/connection refused/i)).toBeInTheDocument());
  });

  // ─── Model select (multiple catalog models) ───────────────────────────────

  it('shows model select when provider has multiple models', async () => {
    const settings = makeSettings({
      provider: 'openai',
      model: 'openai-model-v1',
      providers: [
        makeProvider('openai', {
          requires_api_key: true,
          has_api_key: true,
          models: [
            {
              id: 'openai-model-v1',
              label: 'Model v1',
              default_dimensions: 1536,
              allowed_dimensions: [768, 1536],
            },
            {
              id: 'openai-model-v2',
              label: 'Model v2',
              default_dimensions: 3072,
              allowed_dimensions: [3072],
            },
          ],
        }),
      ],
    });
    vi.mocked(loadEmbeddingsSettings).mockResolvedValue(settings);

    renderWithProviders(<EmbeddingsPanel />);

    const modelSelect = await screen.findByRole('combobox', { name: /model/i });
    expect(modelSelect).toHaveValue('openai-model-v1');
    expect(screen.getByText(/model v2/i)).toBeInTheDocument();
  });

  it('changing model select calls handleModelChange', async () => {
    const settings = makeSettings({
      provider: 'openai',
      model: 'openai-model-v1',
      providers: [
        makeProvider('openai', {
          requires_api_key: true,
          has_api_key: true,
          models: [
            {
              id: 'openai-model-v1',
              label: 'Model v1',
              default_dimensions: 1536,
              allowed_dimensions: [768, 1536],
            },
            {
              id: 'openai-model-v2',
              label: 'Model v2',
              default_dimensions: 3072,
              allowed_dimensions: [3072],
            },
          ],
        }),
      ],
    });
    vi.mocked(loadEmbeddingsSettings).mockResolvedValue(settings);
    vi.mocked(updateEmbeddingsSettings).mockResolvedValue({ model: 'openai-model-v2' });

    renderWithProviders(<EmbeddingsPanel />);

    const modelSelect = await screen.findByRole('combobox', { name: /model/i });
    fireEvent.change(modelSelect, { target: { value: 'openai-model-v2' } });

    await waitFor(() =>
      expect(vi.mocked(updateEmbeddingsSettings)).toHaveBeenCalledWith(
        expect.objectContaining({ model: 'openai-model-v2', confirm_wipe: false })
      )
    );
  });

  // ─── Dimensions select ────────────────────────────────────────────────────

  it('shows dimensions select when provider model allows multiple dimensions', async () => {
    const settings = makeSettings({
      provider: 'openai',
      model: 'openai-model-v1',
      dimensions: 1536,
      providers: [
        makeProvider('openai', {
          requires_api_key: true,
          has_api_key: true,
          models: [
            {
              id: 'openai-model-v1',
              label: 'Model v1',
              default_dimensions: 1536,
              allowed_dimensions: [768, 1536],
            },
          ],
        }),
      ],
    });
    vi.mocked(loadEmbeddingsSettings).mockResolvedValue(settings);

    renderWithProviders(<EmbeddingsPanel />);

    const dimsSelect = await screen.findByRole('combobox', { name: /dimensions/i });
    expect(dimsSelect).toHaveValue('1536');
  });

  it('changing dimensions select calls handleDimsChange', async () => {
    const settings = makeSettings({
      provider: 'openai',
      model: 'openai-model-v1',
      dimensions: 1536,
      providers: [
        makeProvider('openai', {
          requires_api_key: true,
          has_api_key: true,
          models: [
            {
              id: 'openai-model-v1',
              label: 'Model v1',
              default_dimensions: 1536,
              allowed_dimensions: [768, 1536],
            },
          ],
        }),
      ],
    });
    vi.mocked(loadEmbeddingsSettings).mockResolvedValue(settings);
    vi.mocked(updateEmbeddingsSettings).mockResolvedValue({ dimensions: 768 });

    renderWithProviders(<EmbeddingsPanel />);

    const dimsSelect = await screen.findByRole('combobox', { name: /dimensions/i });
    fireEvent.change(dimsSelect, { target: { value: '768' } });

    await waitFor(() =>
      expect(vi.mocked(updateEmbeddingsSettings)).toHaveBeenCalledWith(
        expect.objectContaining({ dimensions: 768, confirm_wipe: false })
      )
    );
  });

  // ─── "none" provider (vector search disabled banner) ─────────────────────

  it('shows vector search disabled banner when selected provider is none', async () => {
    const settings = makeSettings({
      provider: 'none',
      providers: [
        makeProvider('none', { requires_api_key: false }),
        makeProvider('managed', { requires_api_key: false }),
      ],
    });
    vi.mocked(loadEmbeddingsSettings).mockResolvedValue(settings);

    renderWithProviders(<EmbeddingsPanel />);
    await screen.findByText('None');

    // Vector search disabled warning should appear
    await waitFor(() => expect(screen.getByText(/vector search.*disabled/i)).toBeInTheDocument());
  });

  // ─── Embedded mode ────────────────────────────────────────────────────────

  it('does not render the SettingsHeader in embedded mode', async () => {
    renderWithProviders(<EmbeddingsPanel embedded />);
    await screen.findByText('Managed');

    // No header rendered in embedded mode
    expect(screen.queryByRole('heading', { name: /embeddings/i })).not.toBeInTheDocument();
  });
});
