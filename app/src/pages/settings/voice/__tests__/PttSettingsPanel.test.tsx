import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { I18nProvider } from '../../../../lib/i18n/I18nContext';
import { initialPttState, type PttState } from '../../../../store/pttSlice';
import { renderWithProviders } from '../../../../test/test-utils';
import PttSettingsPanel from '../PttSettingsPanel';

/**
 * Render PttSettingsPanel with the given PTT slice state pre-seeded so
 * each test can assert against a known starting point. We wrap in the
 * real `I18nProvider` so the panel's labels resolve to the en.ts copy
 * — that lets tests query by their final rendered text without
 * hard-coding the message ids.
 */
function renderPanel(pttOverrides: Partial<PttState> = {}) {
  const preloadedState = {
    locale: { current: 'en' as const },
    ptt: { ...initialPttState, ...pttOverrides },
  };
  return renderWithProviders(
    <I18nProvider>
      <PttSettingsPanel />
    </I18nProvider>,
    { preloadedState }
  );
}

describe('PttSettingsPanel', () => {
  it('renders the "not set" hint when no shortcut is bound', () => {
    renderPanel({ shortcut: null });
    expect(screen.getByText(/Push-to-talk is off: pick a hotkey to enable\./i)).toBeInTheDocument();
  });

  it('renders the bound shortcut when set', () => {
    renderPanel({ shortcut: 'F13' });
    expect(screen.getByTestId('ptt-shortcut-input')).toHaveValue('F13');
    // The unset hint should NOT show once a shortcut is bound.
    expect(
      screen.queryByText(/Push-to-talk is off: pick a hotkey to enable\./i)
    ).not.toBeInTheDocument();
  });

  it('toggles speakReplies via the switch', () => {
    const { store } = renderPanel({ shortcut: 'F13', speakReplies: true });
    const speakSwitch = screen.getByTestId('ptt-speak-replies-switch');
    expect(speakSwitch).toHaveAttribute('aria-checked', 'true');

    fireEvent.click(speakSwitch);

    const stateAfter = (store.getState() as { ptt: PttState }).ptt;
    expect(stateAfter.speakReplies).toBe(false);
    // And the aria-checked attribute should flip on the rendered switch.
    expect(screen.getByTestId('ptt-speak-replies-switch')).toHaveAttribute('aria-checked', 'false');
  });

  it('toggles showOverlay via the switch', () => {
    const { store } = renderPanel({ shortcut: 'F13', showOverlay: true });
    const overlaySwitch = screen.getByTestId('ptt-show-overlay-switch');
    expect(overlaySwitch).toHaveAttribute('aria-checked', 'true');

    fireEvent.click(overlaySwitch);

    const stateAfter = (store.getState() as { ptt: PttState }).ptt;
    expect(stateAfter.showOverlay).toBe(false);
  });

  it('updates the shortcut when a key is captured in the input', () => {
    const { store } = renderPanel({ shortcut: null });
    const input = screen.getByTestId('ptt-shortcut-input');

    // Simulate a real keyboard event — the panel listens for keydown on the
    // focused input and captures the key code (e.g. "F13"). Using fireEvent
    // because userEvent.keyboard treats F13 as a sequence.
    fireEvent.keyDown(input, { key: 'F13', code: 'F13' });

    const stateAfter = (store.getState() as { ptt: PttState }).ptt;
    expect(stateAfter.shortcut).toBe('F13');
  });

  it('shows the panel title and description from the en locale', () => {
    renderPanel({ shortcut: null });
    expect(screen.getByText('Push-to-talk')).toBeInTheDocument();
    expect(screen.getByText(/Hold a key to talk to OpenHuman/i)).toBeInTheDocument();
  });

  it('renders the localized registration error when the slice has a dictation conflict', () => {
    renderPanel({
      shortcut: 'F13',
      registrationError: "ptt shortcut 'F13' conflicts with the dictation hotkey",
    });
    const errEl = screen.getByTestId('ptt-registration-error');
    expect(errEl).toBeInTheDocument();
    expect(errEl).toHaveTextContent(/already used by dictation/i);
  });

  it('renders a localized Wayland error when the slice has one', () => {
    renderPanel({
      shortcut: 'F13',
      registrationError: 'global shortcuts are not supported in this Wayland session',
    });
    expect(screen.getByTestId('ptt-registration-error')).toHaveTextContent(/wayland/i);
  });

  it('renders the raw error string for unrecognised errors', () => {
    renderPanel({ shortcut: 'F13', registrationError: 'some unexpected Tauri error' });
    expect(screen.getByTestId('ptt-registration-error')).toHaveTextContent(
      'some unexpected Tauri error'
    );
  });

  it('does not render a registration error when registrationError is null', () => {
    renderPanel({ shortcut: 'F13', registrationError: null });
    expect(screen.queryByTestId('ptt-registration-error')).not.toBeInTheDocument();
  });

  it('hides the registration error when a captureError (modifier-only) is also present', () => {
    // Both errors at once — captureError wins because it's more immediate.
    // Trigger captureError by pressing a modifier-only key.
    renderPanel({ shortcut: 'F13', registrationError: 'some unexpected Tauri error' });
    const input = screen.getByTestId('ptt-shortcut-input');
    fireEvent.keyDown(input, { key: 'Shift', code: 'ShiftLeft', shiftKey: true });
    // captureError is now set — registration error should be hidden.
    expect(screen.queryByTestId('ptt-registration-error')).not.toBeInTheDocument();
    expect(screen.getByTestId('ptt-shortcut-error')).toBeInTheDocument();
  });
});
