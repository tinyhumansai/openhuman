import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { REHYDRATE } from 'redux-persist';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import mascotReducer, {
  DEFAULT_MASCOT_COLOR,
  setCustomMascotGifUrl,
  setMascotColor,
  setMascotVoiceId,
  setSecondaryMascotId,
  setSelectedMascotId,
} from '../../../../store/mascotSlice';
import MascotPanel from '../MascotPanel';

const { mockNavigateBack, useMascotManifestMock, mockSynthesizeSpeech } = vi.hoisted(() => ({
  mockNavigateBack: vi.fn(),
  useMascotManifestMock: vi.fn(),
  mockSynthesizeSpeech: vi.fn(),
}));

vi.mock('../../../../features/human/Mascot/manifest/useMascotManifest', () => ({
  useMascotManifest: () => useMascotManifestMock(),
}));

vi.mock('../../../../features/human/voice/ttsClient', () => ({
  synthesizeSpeech: (...args: unknown[]) => mockSynthesizeSpeech(...args),
}));

vi.mock('../../../../features/human/Mascot', async importOriginal => {
  const actual = await importOriginal<typeof import('../../../../features/human/Mascot')>();
  return {
    ...actual,
    RiveMascot: () => <div data-testid="rive-mascot-preview" />,
    ManifestRiveMascot: ({ entry }: { entry: { id: string } }) => (
      <div data-testid={`manifest-mascot-preview-${entry.id}`} />
    ),
    CustomGifMascot: ({ src }: { src: string }) => (
      <img data-testid="custom-gif-mascot" src={src} alt="" />
    ),
  };
});

// Build a minimal manifest entry for the picker list / preview.
function manifestEntry(id: string, name: string, status: 'ready' | 'draft' = 'ready') {
  return {
    id,
    name,
    description: '',
    status,
    tags: [],
    stateEngine: {
      idlePoseCycle: ['idle', 'dancing'],
      states: { idle: 'idle', thinking: 'thinking' },
      visemeCodes: ['sil', 'PP', 'aa'],
    },
    files: [
      { path: `${id}.riv`, bytes: 1, role: 'runtime', sha256: id, url: `https://x/${id}.riv` },
    ],
  };
}

function manifestResult(
  mascots: ReturnType<typeof manifestEntry>[],
  overrides: Partial<{
    manifest: unknown;
    entry: unknown;
    loading: boolean;
    error: Error | null;
  }> = {}
) {
  const manifest =
    'manifest' in overrides
      ? overrides.manifest
      : {
          schemaVersion: 1,
          generatedAt: '',
          mascots,
          source: { repository: '', branch: '', commit: '' },
        };
  return {
    manifest,
    entry: 'entry' in overrides ? overrides.entry : (mascots[0] ?? null),
    loading: overrides.loading ?? false,
    error: overrides.error ?? null,
  };
}

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: mockNavigateBack,
    breadcrumbs: [{ label: 'Settings' }],
  }),
}));

function buildStore() {
  return configureStore({ reducer: { mascot: mascotReducer } });
}

function renderPanel(store = buildStore()) {
  return {
    store,
    ...render(
      <Provider store={store}>
        <MemoryRouter>
          <MascotPanel />
        </MemoryRouter>
      </Provider>
    ),
  };
}

describe('MascotPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useMascotManifestMock.mockReturnValue(manifestResult([]));
    mockSynthesizeSpeech.mockResolvedValue(new Uint8Array(0));
  });

  it('renders a radio swatch for each supported color', () => {
    renderPanel();
    expect(screen.getByRole('radiogroup', { name: 'OpenHuman color' })).toBeInTheDocument();
    for (const label of ['Yellow', 'Burgundy', 'Black', 'Navy', 'Custom']) {
      expect(screen.getByRole('radio', { name: label })).toBeInTheDocument();
    }
  });

  it('marks the currently selected color as aria-checked', () => {
    const store = buildStore();
    store.dispatch(setMascotColor('navy'));
    renderPanel(store);
    expect(screen.getByRole('radio', { name: 'Navy' })).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByRole('radio', { name: 'Yellow' })).toHaveAttribute('aria-checked', 'false');
  });

  it('dispatches setMascotColor when a swatch is clicked', () => {
    const { store } = renderPanel();
    fireEvent.click(screen.getByRole('radio', { name: 'Burgundy' }));
    expect(store.getState().mascot.color).toBe('burgundy');
  });

  it('is a no-op when clicking the already-selected color', () => {
    const store = buildStore();
    store.dispatch(setMascotColor('custom'));
    const dispatchSpy = vi.spyOn(store, 'dispatch');
    renderPanel(store);
    fireEvent.click(screen.getByRole('radio', { name: 'Custom' }));
    // No additional dispatches beyond what React-Redux did to subscribe.
    expect(dispatchSpy).not.toHaveBeenCalled();
    expect(store.getState().mascot.color).toBe('custom');
  });

  it('invokes navigateBack from the header back button', () => {
    renderPanel();
    fireEvent.click(screen.getByLabelText('Back'));
    expect(mockNavigateBack).toHaveBeenCalledTimes(1);
  });
});

// Batch-5: rehydrate cases + unknown-color fallback (issue#1651, pr#1667)
describe('MascotPanel — mascotSlice rehydrate guard', () => {
  it('restores a known persisted color from a REHYDRATE action', () => {
    const store = configureStore({ reducer: { mascot: mascotReducer } });
    store.dispatch({ type: REHYDRATE, key: 'mascot', payload: { color: 'burgundy' } });
    expect(store.getState().mascot.color).toBe('burgundy');
  });

  it('falls back to yellow when REHYDRATE contains an unknown color string', () => {
    const store = configureStore({ reducer: { mascot: mascotReducer } });
    store.dispatch({ type: REHYDRATE, key: 'mascot', payload: { color: 'hot-pink' } });
    expect(store.getState().mascot.color).toBe(DEFAULT_MASCOT_COLOR);
  });

  it('falls back to yellow when REHYDRATE payload is missing the color field', () => {
    const store = configureStore({ reducer: { mascot: mascotReducer } });
    store.dispatch({ type: REHYDRATE, key: 'mascot', payload: {} });
    expect(store.getState().mascot.color).toBe(DEFAULT_MASCOT_COLOR);
  });

  it('falls back to yellow when REHYDRATE payload is null', () => {
    const store = configureStore({ reducer: { mascot: mascotReducer } });
    store.dispatch({ type: REHYDRATE, key: 'mascot', payload: null });
    expect(store.getState().mascot.color).toBe(DEFAULT_MASCOT_COLOR);
  });

  it('ignores REHYDRATE actions for other slice keys', () => {
    const store = configureStore({ reducer: { mascot: mascotReducer } });
    store.dispatch(setMascotColor('navy'));
    store.dispatch({ type: REHYDRATE, key: 'someOtherSlice', payload: { color: 'custom' } });
    // Should remain navy — we only handle key === 'mascot'.
    expect(store.getState().mascot.color).toBe('navy');
  });

  it('renders the rehydrated color as selected in the panel', () => {
    const store = configureStore({ reducer: { mascot: mascotReducer } });
    store.dispatch({ type: REHYDRATE, key: 'mascot', payload: { color: 'custom' } });
    render(
      <Provider store={store}>
        <MemoryRouter>
          <MascotPanel />
        </MemoryRouter>
      </Provider>
    );
    expect(screen.getByRole('radio', { name: 'Custom' })).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByRole('radio', { name: 'Yellow' })).toHaveAttribute('aria-checked', 'false');
  });

  describe('mascot manifest library', () => {
    const yellow = manifestEntry('yellow', 'Yellow');
    const toshi = manifestEntry('toshi', 'Toshi', 'draft');

    it('renders the picker entries from the manifest', () => {
      useMascotManifestMock.mockReturnValue(manifestResult([yellow, toshi]));
      renderPanel();
      expect(screen.getByTestId('manifest-mascot-yellow')).toBeInTheDocument();
      expect(screen.getByTestId('manifest-mascot-toshi')).toBeInTheDocument();
      // Draft status badge surfaces for non-ready mascots.
      expect(screen.getByText('Draft')).toBeInTheDocument();
      // Default-row (local) sentinel
      expect(screen.getByText(/Local OpenHuman/)).toBeInTheDocument();
    });

    it('shows a friendly empty state when the library is empty', () => {
      useMascotManifestMock.mockReturnValue(
        manifestResult([], {
          manifest: {
            schemaVersion: 1,
            generatedAt: '',
            mascots: [],
            source: { repository: '', branch: '', commit: '' },
          },
        })
      );
      renderPanel();
      expect(screen.getByText(/No OpenHuman characters are available yet/i)).toBeInTheDocument();
    });

    it('shows an error when the manifest fails to load', () => {
      useMascotManifestMock.mockReturnValue(
        manifestResult([], { manifest: null, entry: null, error: new Error('offline') })
      );
      renderPanel();
      expect(screen.getByText(/OpenHuman library unavailable: offline/i)).toBeInTheDocument();
    });

    it('dispatches setSelectedMascotId when a mascot is picked', () => {
      useMascotManifestMock.mockReturnValue(manifestResult([yellow]));
      const { store } = renderPanel();
      fireEvent.click(screen.getByTestId('manifest-mascot-yellow'));
      expect(store.getState().mascot.selectedMascotId).toBe('yellow');
    });

    it('previews the active manifest mascot', () => {
      const store = buildStore();
      store.dispatch(setSelectedMascotId('yellow'));
      useMascotManifestMock.mockReturnValue(manifestResult([yellow], { entry: yellow }));
      renderPanel(store);
      expect(screen.getByTestId('manifest-mascot-preview-yellow')).toBeInTheDocument();
    });

    it('clearing the selection returns to the local default', () => {
      const store = buildStore();
      store.dispatch(setSelectedMascotId('yellow'));
      useMascotManifestMock.mockReturnValue(manifestResult([yellow], { entry: yellow }));
      renderPanel(store);
      fireEvent.click(screen.getByText(/Local OpenHuman/));
      expect(store.getState().mascot.selectedMascotId).toBeNull();
    });

    it('saves a custom GIF avatar and previews it', () => {
      const { store } = renderPanel();
      fireEvent.change(screen.getByTestId('mascot-custom-gif-input'), {
        target: { value: '  https://example.com/avatar.gif  ' },
      });
      fireEvent.click(screen.getByTestId('mascot-custom-gif-save'));

      expect(store.getState().mascot.customMascotGifUrl).toBe('https://example.com/avatar.gif');
      expect(screen.getByTestId('custom-gif-mascot')).toHaveAttribute(
        'src',
        'https://example.com/avatar.gif'
      );
    });

    it('rejects non-GIF avatar sources in the panel', () => {
      const { store } = renderPanel();
      fireEvent.change(screen.getByTestId('mascot-custom-gif-input'), {
        target: { value: 'https://example.com/avatar.svg' },
      });
      fireEvent.click(screen.getByTestId('mascot-custom-gif-save'));

      expect(store.getState().mascot.customMascotGifUrl).toBeNull();
      expect(screen.getByTestId('mascot-custom-gif-error')).toHaveTextContent('HTTPS .gif');
    });

    it('selecting a mascot clears the custom GIF avatar', () => {
      const store = buildStore();
      store.dispatch(setCustomMascotGifUrl('https://example.com/avatar.gif'));
      useMascotManifestMock.mockReturnValue(manifestResult([yellow]));
      renderPanel(store);
      fireEvent.click(screen.getByTestId('manifest-mascot-yellow'));

      expect(store.getState().mascot.selectedMascotId).toBe('yellow');
      expect(store.getState().mascot.customMascotGifUrl).toBeNull();
    });
  });
});

// ── Voice picker: save-paste button disabled state (line 525) ────────────────
describe('MascotPanel — voice picker custom voice input (line 525)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useMascotManifestMock.mockReturnValue(manifestResult([]));
    mockSynthesizeSpeech.mockResolvedValue(new Uint8Array(0));
  });

  it('shows save-paste button when a non-curated (custom) voice id is stored', () => {
    // A non-curated voice id triggers isCustomVoice=true automatically
    // without needing to select __custom__ in the picker.
    const store = buildStore();
    store.dispatch(setMascotVoiceId('custom-voice-id-xyz'));
    renderPanel(store);

    // The custom voice input section is visible
    const saveBtn = screen.getByTestId('mascot-voice-save-paste');
    expect(saveBtn).toBeInTheDocument();
  });

  it('save-paste button is disabled when draft matches stored voice id (line 525)', () => {
    const store = buildStore();
    store.dispatch(setMascotVoiceId('custom-voice-id-xyz'));
    renderPanel(store);

    // Draft defaults to storedVoiceId — so draft === storedVoiceId → disabled
    const saveBtn = screen.getByTestId('mascot-voice-save-paste');
    expect(saveBtn).toBeDisabled();
  });

  it('save-paste button is enabled when draft differs from stored voice id (line 525)', () => {
    const store = buildStore();
    store.dispatch(setMascotVoiceId('custom-voice-id-xyz'));
    renderPanel(store);

    const input = screen.getByTestId('mascot-voice-input');
    fireEvent.change(input, { target: { value: 'different-voice-id' } });

    const saveBtn = screen.getByTestId('mascot-voice-save-paste');
    expect(saveBtn).not.toBeDisabled();
  });

  it('clicking save-paste button dispatches new voice id to store', () => {
    const store = buildStore();
    store.dispatch(setMascotVoiceId('custom-voice-id-xyz'));
    renderPanel(store);

    const input = screen.getByTestId('mascot-voice-input');
    fireEvent.change(input, { target: { value: 'new-voice-id' } });

    const saveBtn = screen.getByTestId('mascot-voice-save-paste');
    fireEvent.click(saveBtn);

    expect(store.getState().mascot.voiceId).toBe('new-voice-id');
  });
});

// ── Dual mascots + per-mascot voice (issue #4277) ────────────────────────────
describe('MascotPanel — meeting duo (second mascot)', () => {
  const yellow = manifestEntry('yellow', 'Yellow');
  const toshi = manifestEntry('toshi', 'Toshi');

  beforeEach(() => {
    vi.clearAllMocks();
    useMascotManifestMock.mockReturnValue(manifestResult([yellow, toshi]));
    mockSynthesizeSpeech.mockResolvedValue(new Uint8Array(0));
  });

  it('dispatches setSecondaryMascotId when a second mascot is picked', () => {
    const { store } = renderPanel();
    fireEvent.change(screen.getByTestId('mascot-secondary-select'), { target: { value: 'toshi' } });
    expect(store.getState().mascot.secondaryMascotId).toBe('toshi');
  });

  it('clears the second mascot when None is selected', () => {
    const store = buildStore();
    store.dispatch(setSecondaryMascotId('toshi'));
    renderPanel(store);
    fireEvent.change(screen.getByTestId('mascot-secondary-select'), {
      target: { value: '__none__' },
    });
    expect(store.getState().mascot.secondaryMascotId).toBeNull();
  });

  it('disables the primary mascot as a second-mascot option', () => {
    const store = buildStore();
    store.dispatch(setSelectedMascotId('yellow'));
    renderPanel(store);
    const primaryOption = screen
      .getByTestId('mascot-secondary-select')
      .querySelector('option[value="yellow"]') as HTMLOptionElement | null;
    expect(primaryOption).not.toBeNull();
    expect(primaryOption).toBeDisabled();
  });

  it('hides the duo picker when a custom GIF avatar is set', () => {
    const store = buildStore();
    store.dispatch(setCustomMascotGifUrl('https://example.com/avatar.gif'));
    renderPanel(store);
    expect(screen.queryByTestId('mascot-secondary-select')).not.toBeInTheDocument();
  });

  it('dispatches setMascotVoice for the second mascot when its voice changes', () => {
    const store = buildStore();
    store.dispatch(setSelectedMascotId('yellow'));
    store.dispatch(setSecondaryMascotId('toshi'));
    renderPanel(store);

    // The secondary voice row renders once a distinct duo mascot is set.
    const select = screen.getByTestId('mascot-voice-secondary-select');
    fireEvent.change(select, { target: { value: 'pNInz6obpgDQGcFmaJgB' } });

    expect(store.getState().mascot.mascotVoices.toshi).toBe('pNInz6obpgDQGcFmaJgB');
  });

  it('dispatches setMascotVoice for the primary mascot when its voice changes', () => {
    const store = buildStore();
    store.dispatch(setSelectedMascotId('yellow'));
    store.dispatch(setSecondaryMascotId('toshi'));
    renderPanel(store);

    const select = screen.getByTestId('mascot-voice-primary-select');
    fireEvent.change(select, { target: { value: 'pNInz6obpgDQGcFmaJgB' } });

    expect(store.getState().mascot.mascotVoices.yellow).toBe('pNInz6obpgDQGcFmaJgB');
  });

  it('clears a per-mascot voice via the row reset button', () => {
    const store = buildStore();
    store.dispatch(setSelectedMascotId('yellow'));
    store.dispatch(setSecondaryMascotId('toshi'));
    renderPanel(store);

    // Seed an override, then reset it.
    fireEvent.change(screen.getByTestId('mascot-voice-secondary-select'), {
      target: { value: 'pNInz6obpgDQGcFmaJgB' },
    });
    expect(store.getState().mascot.mascotVoices.toshi).toBe('pNInz6obpgDQGcFmaJgB');

    fireEvent.click(screen.getByTestId('mascot-voice-secondary-reset'));
    expect(store.getState().mascot.mascotVoices.toshi).toBeUndefined();
  });
});
