import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import personaReducer from '../../../../store/personaSlice';
import PersonaPanel from '../PersonaPanel';

const {
  mockNavigateBack,
  mockNavigateToSettings,
  readPersonaFileMock,
  writePersonaFileMock,
  resetPersonaFileMock,
} = vi.hoisted(() => ({
  mockNavigateBack: vi.fn(),
  mockNavigateToSettings: vi.fn(),
  readPersonaFileMock: vi.fn(),
  writePersonaFileMock: vi.fn(),
  resetPersonaFileMock: vi.fn(),
}));

vi.mock('../../../../services/api/personaFilesApi', () => ({
  PERSONA_FILE_SOUL: 'SOUL.md',
  readPersonaFile: (...args: unknown[]) => readPersonaFileMock(...args),
  writePersonaFile: (...args: unknown[]) => writePersonaFileMock(...args),
  resetPersonaFile: (...args: unknown[]) => resetPersonaFileMock(...args),
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: mockNavigateBack,
    navigateToSettings: mockNavigateToSettings,
    breadcrumbs: [{ label: 'Settings' }],
  }),
}));

function buildStore() {
  return configureStore({ reducer: { persona: personaReducer } });
}

function renderPanel(store = buildStore()) {
  return {
    store,
    ...render(
      <Provider store={store}>
        <MemoryRouter>
          <PersonaPanel />
        </MemoryRouter>
      </Provider>
    ),
  };
}

const soulFile = (overrides: Record<string, unknown> = {}) => ({
  filename: 'SOUL.md',
  contents: 'You are helpful.',
  is_default: true,
  path: '/ws/SOUL.md',
  ...overrides,
});

describe('PersonaPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    readPersonaFileMock.mockResolvedValue(soulFile());
    writePersonaFileMock.mockImplementation((_name: string, contents: string) =>
      Promise.resolve(soulFile({ contents, is_default: false }))
    );
    resetPersonaFileMock.mockResolvedValue(
      soulFile({ contents: 'default soul', is_default: true })
    );
  });

  it('loads SOUL.md contents into the editor on mount', async () => {
    renderPanel();
    await waitFor(() => {
      expect(screen.getByTestId('persona-soul-editor')).toHaveValue('You are helpful.');
    });
    expect(readPersonaFileMock).toHaveBeenCalledWith('SOUL.md');
  });

  it('persists the display name to the store on save', async () => {
    const { store } = renderPanel();
    await waitFor(() => expect(screen.getByTestId('persona-soul-editor')).toBeInTheDocument());

    const input = screen.getByTestId('persona-display-name-input');
    fireEvent.change(input, { target: { value: 'Nova' } });
    fireEvent.click(screen.getByTestId('persona-identity-save'));

    expect(store.getState().persona.displayName).toBe('Nova');
  });

  it('keeps the identity save button disabled until a field changes', async () => {
    renderPanel();
    await waitFor(() => expect(screen.getByTestId('persona-soul-editor')).toBeInTheDocument());
    expect(screen.getByTestId('persona-identity-save')).toBeDisabled();
  });

  it('writes edited SOUL.md contents over RPC', async () => {
    renderPanel();
    await waitFor(() => expect(screen.getByTestId('persona-soul-editor')).toBeInTheDocument());

    fireEvent.change(screen.getByTestId('persona-soul-editor'), {
      target: { value: 'You are calm and concise.' },
    });
    fireEvent.click(screen.getByTestId('persona-soul-save'));

    await waitFor(() => {
      expect(writePersonaFileMock).toHaveBeenCalledWith('SOUL.md', 'You are calm and concise.');
    });
  });

  it('resets SOUL.md to the bundled default', async () => {
    // Start from a non-default file so the Reset button is enabled.
    readPersonaFileMock.mockResolvedValue(soulFile({ contents: 'custom', is_default: false }));
    renderPanel();
    await waitFor(() => {
      expect(screen.getByTestId('persona-soul-editor')).toHaveValue('custom');
    });

    fireEvent.click(screen.getByTestId('persona-soul-reset'));

    await waitFor(() => {
      expect(resetPersonaFileMock).toHaveBeenCalledWith('SOUL.md');
      expect(screen.getByTestId('persona-soul-editor')).toHaveValue('default soul');
    });
  });

  it('disables Reset while the file is already the bundled default', async () => {
    renderPanel();
    await waitFor(() => expect(screen.getByTestId('persona-soul-editor')).toBeInTheDocument());
    expect(screen.getByTestId('persona-soul-reset')).toBeDisabled();
    expect(screen.getByTestId('persona-soul-default-badge')).toBeInTheDocument();
  });

  it('surfaces a load error', async () => {
    readPersonaFileMock.mockRejectedValue(new Error('boom'));
    renderPanel();
    await waitFor(() => {
      expect(screen.getByTestId('persona-soul-error')).toHaveTextContent('boom');
    });
  });

  it('navigates to mascot settings for avatar & voice', async () => {
    renderPanel();
    await waitFor(() => expect(screen.getByTestId('persona-soul-editor')).toBeInTheDocument());
    fireEvent.click(screen.getByTestId('persona-open-mascot'));
    expect(mockNavigateToSettings).toHaveBeenCalledWith('mascot');
  });
});
