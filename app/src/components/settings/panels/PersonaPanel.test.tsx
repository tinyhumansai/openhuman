import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import PersonaPanel from './PersonaPanel';

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

vi.mock('../../../services/api/personaFilesApi', () => ({
  PERSONA_FILE_SOUL: 'SOUL.md',
  readPersonaFile: (...args: unknown[]) => readPersonaFileMock(...args),
  writePersonaFile: (...args: unknown[]) => writePersonaFileMock(...args),
  resetPersonaFile: (...args: unknown[]) => resetPersonaFileMock(...args),
}));

vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: mockNavigateBack,
    navigateToSettings: mockNavigateToSettings,
    breadcrumbs: [{ label: 'Settings' }],
  }),
}));

const soulFile = (overrides: Record<string, unknown> = {}) => ({
  filename: 'SOUL.md',
  contents: 'You are helpful.',
  is_default: true,
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
    renderWithProviders(<PersonaPanel />);
    await waitFor(() => {
      expect(screen.getByTestId('persona-soul-editor')).toHaveValue('You are helpful.');
    });
    expect(readPersonaFileMock).toHaveBeenCalledWith('SOUL.md');
  });

  it('persists the display name to the store on save', async () => {
    const { store } = renderWithProviders(<PersonaPanel />);
    await waitFor(() => expect(screen.getByTestId('persona-soul-editor')).toBeInTheDocument());

    fireEvent.change(screen.getByTestId('persona-display-name-input'), {
      target: { value: 'Nova' },
    });
    fireEvent.change(screen.getByTestId('persona-description-input'), {
      target: { value: 'Calm and concise.' },
    });
    fireEvent.click(screen.getByTestId('persona-identity-save'));

    expect(store.getState().persona.displayName).toBe('Nova');
    expect(store.getState().persona.description).toBe('Calm and concise.');
  });

  it('keeps the identity save button disabled until a field changes', async () => {
    renderWithProviders(<PersonaPanel />);
    await waitFor(() => expect(screen.getByTestId('persona-soul-editor')).toBeInTheDocument());
    expect(screen.getByTestId('persona-identity-save')).toBeDisabled();
  });

  it('writes edited SOUL.md contents over RPC', async () => {
    renderWithProviders(<PersonaPanel />);
    await waitFor(() => expect(screen.getByTestId('persona-soul-editor')).toBeInTheDocument());

    fireEvent.change(screen.getByTestId('persona-soul-editor'), {
      target: { value: 'You are calm and concise.' },
    });
    fireEvent.click(screen.getByTestId('persona-soul-save'));

    await waitFor(() => {
      expect(writePersonaFileMock).toHaveBeenCalledWith('SOUL.md', 'You are calm and concise.');
    });
  });

  it('surfaces a save error when the write RPC fails', async () => {
    writePersonaFileMock.mockRejectedValue(new Error('disk full'));
    renderWithProviders(<PersonaPanel />);
    await waitFor(() => expect(screen.getByTestId('persona-soul-editor')).toBeInTheDocument());

    fireEvent.change(screen.getByTestId('persona-soul-editor'), { target: { value: 'edited' } });
    fireEvent.click(screen.getByTestId('persona-soul-save'));

    await waitFor(() => {
      expect(screen.getByTestId('persona-soul-error')).toHaveTextContent('disk full');
    });
  });

  it('surfaces a reset error when the reset RPC fails', async () => {
    readPersonaFileMock.mockResolvedValue(soulFile({ contents: 'custom', is_default: false }));
    resetPersonaFileMock.mockRejectedValue(new Error('reset boom'));
    renderWithProviders(<PersonaPanel />);
    await waitFor(() => expect(screen.getByTestId('persona-soul-editor')).toHaveValue('custom'));

    fireEvent.click(screen.getByTestId('persona-soul-reset'));

    await waitFor(() => {
      expect(screen.getByTestId('persona-soul-error')).toHaveTextContent('reset boom');
    });
  });

  it('resets SOUL.md to the bundled default', async () => {
    // Start from a non-default file so the Reset button is enabled.
    readPersonaFileMock.mockResolvedValue(soulFile({ contents: 'custom', is_default: false }));
    renderWithProviders(<PersonaPanel />);
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
    renderWithProviders(<PersonaPanel />);
    await waitFor(() => expect(screen.getByTestId('persona-soul-editor')).toBeInTheDocument());
    expect(screen.getByTestId('persona-soul-reset')).toBeDisabled();
    expect(screen.getByTestId('persona-soul-default-badge')).toBeInTheDocument();
  });

  it('surfaces a load error', async () => {
    readPersonaFileMock.mockRejectedValue(new Error('boom'));
    renderWithProviders(<PersonaPanel />);
    await waitFor(() => {
      expect(screen.getByTestId('persona-soul-error')).toHaveTextContent('boom');
    });
  });

  it('navigates to mascot settings for avatar & voice', async () => {
    renderWithProviders(<PersonaPanel />);
    await waitFor(() => expect(screen.getByTestId('persona-soul-editor')).toBeInTheDocument());
    fireEvent.click(screen.getByTestId('persona-open-mascot'));
    expect(mockNavigateToSettings).toHaveBeenCalledWith('mascot');
  });
});
