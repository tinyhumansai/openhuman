/**
 * Tests for the Phase 6 face-mode toggle in the Accounts (Assistant) page.
 *
 * Verifies:
 *  - Face toggle button is rendered
 *  - Face mode is off by default
 *  - Clicking the toggle shows the face-mode panel (data-testid="face-mode-panel")
 *  - Clicking the toggle again hides the face-mode panel
 *  - Face mode state is persisted to localStorage (chat.faceMode)
 *  - When face mode is on, Conversations is rendered with variant="sidebar"
 */
import { configureStore } from '@reduxjs/toolkit';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import accountsReducer from '../../store/accountsSlice';
import chatRuntimeReducer from '../../store/chatRuntimeSlice';
import mascotReducer from '../../store/mascotSlice';
import threadReducer from '../../store/threadSlice';
// ── Static component import (after mocks are hoisted) ───────────────────────
import Accounts from '../Accounts';

// ── Heavy dependency stubs — all must be declared before the component import ──

// Stub Conversations so it doesn't pull in the full chat stack.
vi.mock('../Conversations', () => ({
  default: ({ variant }: { variant?: string }) => (
    <div data-testid="conversations-stub" data-variant={variant ?? 'page'} />
  ),
  AgentChatPanel: () => <div data-testid="agent-chat-panel-stub" />,
}));

// Stub webview account components.
vi.mock('../../components/accounts/WebviewHost', () => ({
  default: () => <div data-testid="webview-host-stub" />,
}));
vi.mock('../../components/accounts/AddAccountModal', () => ({ default: () => null }));
vi.mock('../../components/accounts/providerIcons', () => ({
  AgentIcon: ({ className }: { className?: string }) => (
    <svg data-testid="agent-icon" className={className} />
  ),
  ProviderIcon: ({ provider, className }: { provider: string; className?: string }) => (
    <svg data-testid={`provider-icon-${provider}`} className={className} />
  ),
}));

// Stub webview account service.
vi.mock('../../services/webviewAccountService', () => ({
  startWebviewAccountService: vi.fn(),
  hideWebviewAccount: vi.fn(),
  showWebviewAccount: vi.fn(),
  purgeWebviewAccount: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('../../services/analytics', () => ({ trackEvent: vi.fn() }));

// Stub mascot subcomponents — they pull in a Rive WASM runtime.
vi.mock('../../features/human/Mascot', () => ({
  RiveMascot: () => <div data-testid="rive-mascot-stub" />,
  CustomGifMascot: ({ src }: { src: string }) => (
    <img data-testid="custom-gif-mascot-stub" src={src} alt="" />
  ),
  MascotChipAvatar: () => <span data-testid="mascot-chip-avatar-stub" />,
  getMascotPalette: vi.fn(() => ({ bodyFill: '#4A83DD', neckShadowColor: '#2A63BD' })),
  hexToArgbInt: vi.fn((_hex: string) => 0xff4a83dd),
}));

vi.mock('../../features/human/useHumanMascot', () => ({
  useHumanMascot: () => ({ face: 'idle', visemeCode: 0 }),
}));

vi.mock('../../hooks/usePrewarmMostRecentAccount', () => ({
  usePrewarmMostRecentAccount: vi.fn(),
}));

// ── Helpers ──────────────────────────────────────────────────────────────────

const FACE_MODE_KEY = 'chat.faceMode';

function buildStore() {
  return configureStore({
    reducer: {
      accounts: accountsReducer,
      mascot: mascotReducer,
      thread: threadReducer,
      chatRuntime: chatRuntimeReducer,
    },
  });
}

function renderAccounts(store = buildStore()) {
  return render(
    <Provider store={store}>
      <MemoryRouter>
        <Accounts />
      </MemoryRouter>
    </Provider>
  );
}

// ── Tests ────────────────────────────────────────────────────────────────────

describe('Accounts — face-mode toggle', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  afterEach(() => {
    localStorage.clear();
  });

  it('renders the face-toggle button', () => {
    renderAccounts();
    expect(screen.getByTestId('face-toggle-button')).toBeInTheDocument();
  });

  it('face mode is off by default (no face-mode-panel rendered)', () => {
    renderAccounts();
    expect(screen.queryByTestId('face-mode-panel')).not.toBeInTheDocument();
  });

  it('clicking the toggle shows the face-mode panel', async () => {
    renderAccounts();
    const toggle = screen.getByTestId('face-toggle-button');
    await act(async () => {
      fireEvent.click(toggle);
    });
    expect(screen.getByTestId('face-mode-panel')).toBeInTheDocument();
  });

  it('clicking the toggle again hides the face-mode panel', async () => {
    renderAccounts();
    const toggle = screen.getByTestId('face-toggle-button');
    await act(async () => {
      fireEvent.click(toggle);
    });
    expect(screen.getByTestId('face-mode-panel')).toBeInTheDocument();
    await act(async () => {
      fireEvent.click(screen.getByTestId('face-toggle-button'));
    });
    expect(screen.queryByTestId('face-mode-panel')).not.toBeInTheDocument();
  });

  it('persists face-mode ON to localStorage', async () => {
    renderAccounts();
    await act(async () => {
      fireEvent.click(screen.getByTestId('face-toggle-button'));
    });
    expect(localStorage.getItem(FACE_MODE_KEY)).toBe('1');
  });

  it('persists face-mode OFF to localStorage after toggling twice', async () => {
    renderAccounts();
    await act(async () => {
      fireEvent.click(screen.getByTestId('face-toggle-button'));
    });
    await act(async () => {
      fireEvent.click(screen.getByTestId('face-toggle-button'));
    });
    expect(localStorage.getItem(FACE_MODE_KEY)).toBe('0');
  });

  it('reads face-mode ON from localStorage on mount', () => {
    localStorage.setItem(FACE_MODE_KEY, '1');
    renderAccounts();
    expect(screen.getByTestId('face-mode-panel')).toBeInTheDocument();
  });

  it('when face mode is on, Conversations is rendered with variant="sidebar"', async () => {
    renderAccounts();
    await act(async () => {
      fireEvent.click(screen.getByTestId('face-toggle-button'));
    });
    const conversations = screen.getByTestId('conversations-stub');
    expect(conversations).toHaveAttribute('data-variant', 'sidebar');
  });
});
