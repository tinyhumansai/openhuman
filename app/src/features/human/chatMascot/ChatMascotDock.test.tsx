import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { selectChatMascotDismissed, selectChatMascotExpanded } from '../../../store/mascotSlice';
import { renderWithProviders } from '../../../test/test-utils';
import { ChatMascotProvider } from './ChatMascotContext';
import ChatMascotDock from './ChatMascotDock';

const renderDock = (expanded = false, dismissed = false) =>
  renderWithProviders(
    <ChatMascotProvider>
      <ChatMascotDock />
    </ChatMascotProvider>,
    { preloadedState: { mascot: { chatMascotExpanded: expanded, chatMascotDismissed: dismissed } } }
  );

describe('ChatMascotDock', () => {
  it('renders a labelled toggle inviting the user to talk', () => {
    renderDock(false);

    const dock = screen.getByTestId('chat-mascot-dock');
    expect(dock).toHaveAccessibleName('Talk to your assistant');
    expect(dock).toHaveAttribute('aria-expanded', 'false');
  });

  it('leaves no phantom click target behind once the stage is open', () => {
    // Nothing is painted on the composer while the mascot is on the stage, so a
    // slot left mounted here would be an invisible 64px button.
    renderDock(true);

    expect(screen.queryByTestId('chat-mascot-dock')).not.toBeInTheDocument();
  });

  it('expands the stage on click', () => {
    const { store } = renderDock(false);

    fireEvent.click(screen.getByTestId('chat-mascot-dock'));

    expect(selectChatMascotExpanded(store.getState())).toBe(true);
  });

  it('draws nothing itself — the shared overlay paints over this slot', () => {
    // Guards the single-Rive-instance invariant: if the dock ever grows its own
    // mascot, the app loads the `.riv` twice and the travel becomes a crossfade.
    renderDock(false);

    expect(screen.getByTestId('chat-mascot-dock')).toBeEmptyDOMElement();
  });

  describe('dismiss', () => {
    it('offers a dismiss control alongside the mascot', () => {
      renderDock();
      expect(screen.getByTestId('chat-mascot-dismiss')).toHaveAccessibleName('Hide Tiny');
    });

    it('asks before removing it, and does nothing until confirmed', () => {
      const { store } = renderDock();

      fireEvent.click(screen.getByTestId('chat-mascot-dismiss'));

      expect(screen.getByText('Hide Tiny?')).toBeInTheDocument();
      // Still visible — the click opened a question, not a deletion.
      expect(selectChatMascotDismissed(store.getState())).toBe(false);
      expect(screen.getByTestId('chat-mascot-dock')).toBeInTheDocument();
    });

    it('tells the user where to get it back, using the real menu labels', () => {
      // A control that vanishes with no route back is a trap — and a path that
      // names menu items the UI does not actually have is the same trap with
      // extra steps. These three strings are the live `nav.settings`,
      // `settings.appearance.title` and `settings.appearance.chatHeading`.
      renderDock();
      fireEvent.click(screen.getByTestId('chat-mascot-dismiss'));
      expect(screen.getByText(/Settings › Appearance › Chat/)).toBeInTheDocument();
    });

    it('keeps the mascot when the dialog is cancelled', () => {
      const { store } = renderDock();
      fireEvent.click(screen.getByTestId('chat-mascot-dismiss'));

      fireEvent.click(screen.getByRole('button', { name: 'Keep Tiny' }));

      expect(selectChatMascotDismissed(store.getState())).toBe(false);
      expect(screen.getByTestId('chat-mascot-dock')).toBeInTheDocument();
    });

    it('hides the mascot once confirmed', () => {
      const { store } = renderDock();
      fireEvent.click(screen.getByTestId('chat-mascot-dismiss'));

      fireEvent.click(screen.getByTestId('confirm-dialog-confirm'));

      expect(selectChatMascotDismissed(store.getState())).toBe(true);
      expect(screen.queryByTestId('chat-mascot-dock')).not.toBeInTheDocument();
    });

    it('renders nothing at all once dismissed', () => {
      renderDock(false, true);
      expect(screen.queryByTestId('chat-mascot-dock')).not.toBeInTheDocument();
      expect(screen.queryByTestId('chat-mascot-dismiss')).not.toBeInTheDocument();
    });
  });
});
