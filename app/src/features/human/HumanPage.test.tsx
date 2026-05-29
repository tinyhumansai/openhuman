/**
 * Unit tests for HumanPage — speak-replies localStorage persistence (issue#1520, issue#1502).
 *
 * HumanPage uses a localStorage flag (`human.speakReplies`) to persist the
 * "Speak replies" toggle across sessions.  The default value is `true` when no
 * key is present, `true` when the stored value is `'1'`, and `false` for `'0'`.
 * Toggling the checkbox writes the updated value back to localStorage.
 */
import { configureStore } from '@reduxjs/toolkit';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import chatRuntimeReducer from '../../store/chatRuntimeSlice';
import mascotReducer, { setCustomMascotGifUrl } from '../../store/mascotSlice';
import threadReducer from '../../store/threadSlice';
// ── Static import (after mocks are hoisted) ──────────────────────────────
import HumanPage from './HumanPage';

// ── Heavy dependency stubs ────────────────────────────────────────────────

vi.mock('../../pages/Conversations', () => ({
  default: () => <div data-testid="conversations-stub" />,
}));

vi.mock('../../components/skills/MeetingBotsCard', () => ({
  MeetingBotsModal: ({ onClose }: { onClose: () => void }) => (
    <div role="dialog" aria-label="meeting-bots-modal">
      <button type="button" onClick={onClose}>
        Close modal
      </button>
    </div>
  ),
}));

vi.mock('./Mascot', async importOriginal => {
  const actual = await importOriginal<typeof import('./Mascot')>();
  return {
    ...actual,
    RiveMascot: () => <div data-testid="mascot-stub" />,
    CustomGifMascot: ({ src, face }: { src: string; face?: string }) => (
      <img data-testid="custom-gif-mascot" data-face={face} src={src} alt="" />
    ),
    Ghosty: ({ face, bodyColor }: { face?: string; bodyColor?: string }) => (
      <div data-testid="ghosty-submascot" data-face={face} data-body-color={bodyColor} />
    ),
  };
});

vi.mock('./useHumanMascot', () => ({ useHumanMascot: () => ({ face: 'idle', visemes: [] }) }));

const SPEAK_REPLIES_KEY = 'human.speakReplies';

function buildMinimalStore() {
  return configureStore({
    reducer: { mascot: mascotReducer, thread: threadReducer, chatRuntime: chatRuntimeReducer },
  });
}

function renderHumanPage(store = buildMinimalStore()) {
  return render(
    <Provider store={store}>
      <HumanPage />
    </Provider>
  );
}

describe('HumanPage — speak-replies localStorage persistence', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  afterEach(() => {
    localStorage.clear();
  });

  it('defaults to checked (true) when no localStorage value is set', () => {
    renderHumanPage();
    const checkbox = screen.getByRole('checkbox');
    expect(checkbox).toBeChecked();
  });

  it('reads stored "1" as checked on mount', () => {
    localStorage.setItem(SPEAK_REPLIES_KEY, '1');
    renderHumanPage();
    expect(screen.getByRole('checkbox')).toBeChecked();
  });

  it('reads stored "0" as unchecked on mount', () => {
    localStorage.setItem(SPEAK_REPLIES_KEY, '0');
    renderHumanPage();
    expect(screen.getByRole('checkbox')).not.toBeChecked();
  });

  it('writes "0" to localStorage when the checkbox is unchecked', async () => {
    renderHumanPage();
    const checkbox = screen.getByRole('checkbox');
    expect(checkbox).toBeChecked();

    await act(async () => {
      fireEvent.click(checkbox);
    });

    expect(localStorage.getItem(SPEAK_REPLIES_KEY)).toBe('0');
    expect(checkbox).not.toBeChecked();
  });

  it('writes "1" to localStorage when the checkbox is re-checked', async () => {
    localStorage.setItem(SPEAK_REPLIES_KEY, '0');
    renderHumanPage();
    const checkbox = screen.getByRole('checkbox');
    expect(checkbox).not.toBeChecked();

    await act(async () => {
      fireEvent.click(checkbox);
    });

    expect(localStorage.getItem(SPEAK_REPLIES_KEY)).toBe('1');
    expect(checkbox).toBeChecked();
  });

  it('renders a custom GIF mascot when one is configured', () => {
    const store = buildMinimalStore();
    store.dispatch(setCustomMascotGifUrl('https://example.com/avatar.gif'));

    renderHumanPage(store);

    expect(screen.getByTestId('custom-gif-mascot')).toHaveAttribute(
      'src',
      'https://example.com/avatar.gif'
    );
    expect(screen.queryByTestId('mascot-stub')).not.toBeInTheDocument();
  });
});

describe('HumanPage — join meeting pill', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  afterEach(() => {
    localStorage.clear();
  });

  it('renders the join-meeting pill button', () => {
    renderHumanPage();
    expect(screen.getByTestId('human-join-meeting-pill')).toBeInTheDocument();
  });

  it('opens the MeetingBotsModal when the join-meeting pill is clicked', async () => {
    renderHumanPage();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();

    fireEvent.click(screen.getByTestId('human-join-meeting-pill'));

    expect(screen.getByRole('dialog')).toBeInTheDocument();
  });

  it('closes the MeetingBotsModal when onClose is called', async () => {
    renderHumanPage();
    fireEvent.click(screen.getByTestId('human-join-meeting-pill'));
    expect(screen.getByRole('dialog')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /close modal/i }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
});
