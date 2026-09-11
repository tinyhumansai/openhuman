import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { setCoreStateSnapshot } from '../../../../lib/coreState/store';
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

/**
 * The destructive half of `EmbeddingsPanel`.
 *
 * Changing the embedding model or its dimensionality invalidates every stored
 * vector, so the core answers `EMBEDDINGS_DIMENSION_CHANGE_REQUIRES_WIPE` and
 * the panel must park the change behind a confirmation rather than applying it.
 * That gate is written three times — `handleProviderChange`, `handleModelChange`
 * (panel :207-210) and `handleDimsChange` (:223-226) — and the existing suite
 * exercises only the first. Measured, the panel sat at 70.8% branches with the
 * model and dimension gates, `confirmWipe`'s failure path, and every handler's
 * non-`Error` rejection arm uncovered.
 *
 * What makes these worth pinning individually: the gate is the only thing
 * standing between "user picked a different model from a dropdown" and "every
 * memory embedding is discarded".
 */

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

const WIPE_ERROR = 'EMBEDDINGS_DIMENSION_CHANGE_REQUIRES_WIPE';

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

/** A provider with two models and two dimensionalities, so both selects render. */
const twoModelProvider = () =>
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
  });

const settingsWithTwoModels = (): EmbeddingsSettings => ({
  provider: 'openai',
  model: 'openai-model-v1',
  dimensions: 1536,
  rate_limit_per_min: 60,
  vector_search_enabled: true,
  providers: [twoModelProvider()],
});

function setCoreSession() {
  setCoreStateSnapshot({
    isBootstrapping: false,
    isReady: true,
    snapshot: {
      auth: { isAuthenticated: true, userId: 'u-1', user: null, profileId: 'p-1' },
      sessionToken: 'header.payload.remote',
      currentUser: null,
      onboardingCompleted: true,
      chatOnboardingCompleted: true,
      analyticsEnabled: false,
      localState: { encryptionKey: null, onboardingTasks: null, keyringConsent: null },
      keyringStatus: {
        available: true,
        failureReason: null,
        activeMode: 'os_keyring',
        backendName: 'os',
      },
      runtime: { localAi: null, service: null },
    },
    teams: [],
    teamMembersById: {},
    teamInvitesById: {},
  });
}

const modelSelect = () => screen.findByRole('combobox', { name: /model/i });
const dimsSelect = () => screen.findByRole('combobox', { name: /dimension/i });

beforeEach(() => {
  vi.clearAllMocks();
  setCoreSession();
  vi.mocked(loadEmbeddingsSettings).mockResolvedValue(settingsWithTwoModels());
  vi.mocked(updateEmbeddingsSettings).mockResolvedValue({
    provider: 'openai',
    model: 'openai-model-v1',
    dimensions: 1536,
  });
  vi.mocked(setEmbeddingsApiKey).mockResolvedValue(undefined);
  vi.mocked(clearEmbeddingsApiKey).mockResolvedValue(undefined);
  vi.mocked(testEmbeddingsConnection).mockResolvedValue({
    success: true,
    provider: 'openai',
    model: 'openai-model-v1',
    actual_dimensions: 1536,
  });
});

describe('EmbeddingsPanel — the wipe gate on a model change', () => {
  it('parks a model change behind a confirmation instead of applying it', async () => {
    vi.mocked(updateEmbeddingsSettings).mockResolvedValue({ error: WIPE_ERROR } as never);
    renderWithProviders(<EmbeddingsPanel />);

    fireEvent.change(await modelSelect(), { target: { value: 'openai-model-v2' } });

    await waitFor(() => expect(updateEmbeddingsSettings).toHaveBeenCalledTimes(1));
    // The probing call must be non-destructive...
    expect(updateEmbeddingsSettings).toHaveBeenCalledWith(
      expect.objectContaining({ model: 'openai-model-v2', confirm_wipe: false })
    );
    // ...and the gate must raise a confirmation. This is the assertion that
    // distinguishes "parked" from "silently applied": with the gate removed the
    // call count and arguments are identical, only the dialog is missing.
    expect(
      await screen.findByRole('button', { name: /wipe|confirm|continue/i })
    ).toBeInTheDocument();
    expect(updateEmbeddingsSettings).toHaveBeenCalledTimes(1);
  });

  it("carries the new model's default dimensions into the pending wipe", async () => {
    // Model v2 defaults to 3072. Confirming must apply 3072, not the 1536 the
    // panel was showing — applying the old dimensionality to a new model is how
    // a wipe produces vectors nothing can query.
    vi.mocked(updateEmbeddingsSettings).mockResolvedValueOnce({ error: WIPE_ERROR } as never);
    renderWithProviders(<EmbeddingsPanel />);

    fireEvent.change(await modelSelect(), { target: { value: 'openai-model-v2' } });
    await waitFor(() => expect(updateEmbeddingsSettings).toHaveBeenCalledTimes(1));

    vi.mocked(updateEmbeddingsSettings).mockResolvedValue({
      provider: 'openai',
      model: 'openai-model-v2',
      dimensions: 3072,
    });
    fireEvent.click(await screen.findByRole('button', { name: /wipe|confirm|continue/i }));

    await waitFor(() =>
      expect(updateEmbeddingsSettings).toHaveBeenLastCalledWith(
        expect.objectContaining({ model: 'openai-model-v2', dimensions: 3072, confirm_wipe: true })
      )
    );
  });
});

describe('EmbeddingsPanel — the wipe gate on a dimensions change', () => {
  it('parks a dimensions change behind a confirmation', async () => {
    vi.mocked(updateEmbeddingsSettings).mockResolvedValue({ error: WIPE_ERROR } as never);
    renderWithProviders(<EmbeddingsPanel />);

    fireEvent.change(await dimsSelect(), { target: { value: '768' } });

    await waitFor(() => expect(updateEmbeddingsSettings).toHaveBeenCalledTimes(1));
    expect(updateEmbeddingsSettings).toHaveBeenCalledWith(
      expect.objectContaining({ dimensions: 768, confirm_wipe: false })
    );
    // As above: the dialog is the only observable difference when the gate is
    // removed, so it is what this test turns on.
    expect(
      await screen.findByRole('button', { name: /wipe|confirm|continue/i })
    ).toBeInTheDocument();
  });

  it('confirms a dimensions-only wipe without naming a model', async () => {
    vi.mocked(updateEmbeddingsSettings).mockResolvedValueOnce({ error: WIPE_ERROR } as never);
    renderWithProviders(<EmbeddingsPanel />);

    fireEvent.change(await dimsSelect(), { target: { value: '768' } });
    await waitFor(() => expect(updateEmbeddingsSettings).toHaveBeenCalledTimes(1));

    vi.mocked(updateEmbeddingsSettings).mockResolvedValue({
      provider: 'openai',
      model: 'openai-model-v1',
      dimensions: 768,
    });
    fireEvent.click(await screen.findByRole('button', { name: /wipe|confirm|continue/i }));

    await waitFor(() => expect(updateEmbeddingsSettings).toHaveBeenCalledTimes(2));
    const last = vi.mocked(updateEmbeddingsSettings).mock.calls.at(-1)?.[0] as Record<
      string,
      unknown
    >;
    expect(last).toMatchObject({ dimensions: 768, confirm_wipe: true });
    // A dimensions-only change must not smuggle a model switch into the wipe.
    expect(last).not.toHaveProperty('model');
  });
});

describe('EmbeddingsPanel — failures around the wipe', () => {
  it('surfaces a string rejection from a model change', async () => {
    vi.mocked(updateEmbeddingsSettings).mockRejectedValue('provider rejected the model');
    renderWithProviders(<EmbeddingsPanel />);

    fireEvent.change(await modelSelect(), { target: { value: 'openai-model-v2' } });
    expect(await screen.findByText(/provider rejected the model/)).toBeInTheDocument();
  });

  it('surfaces a string rejection from a dimensions change', async () => {
    vi.mocked(updateEmbeddingsSettings).mockRejectedValue('dimension not supported');
    renderWithProviders(<EmbeddingsPanel />);

    fireEvent.change(await dimsSelect(), { target: { value: '768' } });
    expect(await screen.findByText(/dimension not supported/)).toBeInTheDocument();
  });

  it('surfaces a failure raised while performing the confirmed wipe', async () => {
    vi.mocked(updateEmbeddingsSettings).mockResolvedValueOnce({ error: WIPE_ERROR } as never);
    renderWithProviders(<EmbeddingsPanel />);

    fireEvent.change(await modelSelect(), { target: { value: 'openai-model-v2' } });
    await waitFor(() => expect(updateEmbeddingsSettings).toHaveBeenCalledTimes(1));

    vi.mocked(updateEmbeddingsSettings).mockRejectedValue(new Error('wipe failed midway'));
    fireEvent.click(await screen.findByRole('button', { name: /wipe|confirm|continue/i }));

    // The user must be told, not left looking at a dialog that closed.
    expect(await screen.findByText(/wipe failed midway/)).toBeInTheDocument();
  });

  it('clears the pending wipe once confirmed so a second confirm cannot re-fire it', async () => {
    vi.mocked(updateEmbeddingsSettings).mockResolvedValueOnce({ error: WIPE_ERROR } as never);
    renderWithProviders(<EmbeddingsPanel />);

    fireEvent.change(await modelSelect(), { target: { value: 'openai-model-v2' } });
    const confirm = await screen.findByRole('button', { name: /wipe|confirm|continue/i });

    vi.mocked(updateEmbeddingsSettings).mockResolvedValue({
      provider: 'openai',
      model: 'openai-model-v2',
      dimensions: 3072,
    });
    fireEvent.click(confirm);
    await waitFor(() => expect(updateEmbeddingsSettings).toHaveBeenCalledTimes(2));

    // The dialog is gone, so the destructive call cannot be issued twice.
    await waitFor(() =>
      expect(
        screen.queryByRole('button', { name: /wipe|confirm|continue/i })
      ).not.toBeInTheDocument()
    );
    expect(updateEmbeddingsSettings).toHaveBeenCalledTimes(2);
  });
});
