import { render, screen } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import Accounts from '../Accounts';

const mockDispatch = vi.fn();

const agentState = {
  accounts: {
    accounts: {
      'acct-whatsapp': {
        id: 'acct-whatsapp',
        provider: 'whatsapp',
        label: 'WhatsApp',
        createdAt: '2026-01-01T00:00:00.000Z',
        status: 'open',
      },
    },
    order: ['acct-whatsapp'],
    activeAccountId: '__agent__',
  },
};

let mockState = agentState;

vi.mock('../../hooks/usePrewarmMostRecentAccount', () => ({
  usePrewarmMostRecentAccount: vi.fn(),
}));
vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));
vi.mock('../../services/webviewAccountService', () => ({ startWebviewAccountService: vi.fn() }));
vi.mock('../../store/hooks', () => ({
  useAppDispatch: () => mockDispatch,
  useAppSelector: (selector: (state: typeof agentState) => unknown) => selector(mockState),
}));
vi.mock('../../features/human/Mascot', () => ({
  CustomGifMascot: () => null,
  RiveMascot: () => null,
  getMascotPalette: () => ({ bodyFill: '#000000', neckShadowColor: '#111111' }),
  hexToArgbInt: () => 0,
}));
vi.mock('../../features/human/useHumanMascot', () => ({
  useHumanMascot: () => ({ face: null, visemeCode: null }),
}));
vi.mock('../../store/mascotSlice', () => ({
  selectCustomMascotGifUrl: () => null,
  selectCustomPrimaryColor: () => '#000000',
  selectCustomSecondaryColor: () => '#111111',
  selectMascotColor: () => 'blue',
}));
vi.mock('../Conversations', () => ({
  default: ({ variant }: { variant: string }) => <div data-testid="conversations">{variant}</div>,
  AgentChatPanel: () => <div data-testid="agent-chat-panel" />,
}));

describe('Accounts provider selection', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockState = agentState;
  });

  it('mounts the routed agent chat panel while the agent is selected', () => {
    renderAccounts();

    expect(screen.getByTestId('agent-chat-panel')).toBeInTheDocument();
  });

  it('does not mount the routed agent chat panel while a provider is selected', () => {
    mockState = {
      ...agentState,
      accounts: { ...agentState.accounts, activeAccountId: 'acct-whatsapp' },
    };

    renderAccounts();

    expect(screen.queryByTestId('agent-chat-panel')).not.toBeInTheDocument();
  });

  it('selects the agent account for thread routes', () => {
    renderAccounts('/chat/thread-route-1');

    expect(mockDispatch).toHaveBeenCalledWith({
      type: 'accounts/setActiveAccount',
      payload: '__agent__',
    });
  });
});

function renderAccounts(route = '/chat') {
  return render(
    <MemoryRouter initialEntries={[route]}>
      <Routes>
        <Route path="/chat/:threadId?" element={<Accounts />} />
      </Routes>
    </MemoryRouter>
  );
}
