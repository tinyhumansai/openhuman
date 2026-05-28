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
  setSelectedMascotId,
} from '../../../../store/mascotSlice';
import MascotPanel from '../MascotPanel';

const { mockNavigateBack, fetchMascotListMock, getCachedMascotDetailMock } = vi.hoisted(() => ({
  mockNavigateBack: vi.fn(),
  fetchMascotListMock: vi.fn(),
  getCachedMascotDetailMock: vi.fn(),
}));

vi.mock('../../../../services/mascotService', () => ({
  fetchMascotList: (...args: unknown[]) => fetchMascotListMock(...args),
  getCachedMascotDetail: (...args: unknown[]) => getCachedMascotDetailMock(...args),
}));

vi.mock('../../../../features/human/Mascot', async importOriginal => {
  const actual = await importOriginal<typeof import('../../../../features/human/Mascot')>();
  return {
    ...actual,
    RiveMascot: () => <div data-testid="rive-mascot-preview" />,
    CustomGifMascot: ({ src }: { src: string }) => (
      <img data-testid="custom-gif-mascot" src={src} alt="" />
    ),
  };
});

vi.mock('../../../../features/human/Mascot/backend/BackendMascot', () => ({
  BackendMascot: ({ mascot }: { mascot: { id: string } }) => (
    <div data-testid={`backend-mascot-preview-${mascot.id}`} />
  ),
}));

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
    fetchMascotListMock.mockResolvedValue([]);
    getCachedMascotDetailMock.mockResolvedValue(null);
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

  describe('backend mascot library', () => {
    const summary = {
      id: 'yellow',
      name: 'Yellow',
      version: '1.0.0',
      description: '',
      states: [{ id: 'idle', label: 'Idle', description: '' }],
      hasVisemes: true,
    };
    const detail = {
      id: 'yellow',
      name: 'Yellow',
      version: '1.0.0',
      description: '',
      viewBox: '0 0 1 1',
      defaultState: 'idle',
      variables: [],
      states: [{ id: 'idle', label: 'Idle', description: '', svg: '<svg/>' }],
      visemes: [],
    };

    it('renders the picker entries returned by the API', async () => {
      fetchMascotListMock.mockResolvedValueOnce([summary]);
      renderPanel();
      expect(await screen.findByTestId('backend-mascot-yellow')).toBeInTheDocument();
      // Default-row (local) sentinel
      expect(screen.getByText(/Local OpenHuman/)).toBeInTheDocument();
    });

    it('shows a friendly empty state when the library is empty', async () => {
      fetchMascotListMock.mockResolvedValueOnce([]);
      renderPanel();
      expect(
        await screen.findByText(/No OpenHuman characters are available yet/i)
      ).toBeInTheDocument();
    });

    it('shows an error when the library endpoint rejects', async () => {
      fetchMascotListMock.mockRejectedValueOnce(new Error('offline'));
      renderPanel();
      expect(
        await screen.findByText(/OpenHuman library unavailable: offline/i)
      ).toBeInTheDocument();
    });

    it('dispatches setSelectedMascotId when a backend mascot is picked', async () => {
      fetchMascotListMock.mockResolvedValueOnce([summary]);
      getCachedMascotDetailMock.mockResolvedValueOnce(detail);
      const { store } = renderPanel();
      const row = await screen.findByTestId('backend-mascot-yellow');
      fireEvent.click(row);
      expect(store.getState().mascot.selectedMascotId).toBe('yellow');
    });

    it('loads + previews the active backend mascot detail', async () => {
      const store = buildStore();
      store.dispatch(setSelectedMascotId('yellow'));
      fetchMascotListMock.mockResolvedValueOnce([summary]);
      getCachedMascotDetailMock.mockResolvedValueOnce(detail);
      renderPanel(store);
      expect(await screen.findByTestId('backend-mascot-preview-yellow')).toBeInTheDocument();
      expect(getCachedMascotDetailMock).toHaveBeenCalledWith('yellow');
    });

    it('clearing the selection returns to the local default', async () => {
      const store = buildStore();
      store.dispatch(setSelectedMascotId('yellow'));
      fetchMascotListMock.mockResolvedValueOnce([summary]);
      getCachedMascotDetailMock.mockResolvedValueOnce(detail);
      renderPanel(store);
      const localRow = await screen.findByText(/Local OpenHuman/);
      fireEvent.click(localRow);
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

    it('selecting a backend mascot clears the custom GIF avatar', async () => {
      const store = buildStore();
      store.dispatch(setCustomMascotGifUrl('https://example.com/avatar.gif'));
      fetchMascotListMock.mockResolvedValueOnce([summary]);
      renderPanel(store);
      fireEvent.click(await screen.findByTestId('backend-mascot-yellow'));

      expect(store.getState().mascot.selectedMascotId).toBe('yellow');
      expect(store.getState().mascot.customMascotGifUrl).toBeNull();
    });
  });
});
