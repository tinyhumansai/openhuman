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

// ── Static import (after mocks are hoisted) ──────────────────────────────
import HumanPage from './HumanPage';

// ── Heavy dependency stubs ────────────────────────────────────────────────

vi.mock('../../pages/Conversations', () => ({
  default: () => <div data-testid="conversations-stub" />,
}));

vi.mock('./Mascot', () => ({ YellowMascot: () => <div data-testid="mascot-stub" /> }));

vi.mock('./useHumanMascot', () => ({ useHumanMascot: () => ({ face: 'idle', visemes: [] }) }));

vi.mock('../../store/hooks', () => ({ useAppSelector: () => 'yellow' }));

vi.mock('../../store/mascotSlice', () => ({ selectMascotColor: () => 'yellow' }));

const SPEAK_REPLIES_KEY = 'human.speakReplies';

function buildMinimalStore() {
  return configureStore({ reducer: { _noop: (_s: null = null) => _s } });
}

function renderHumanPage() {
  const store = buildMinimalStore();
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
});
