import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import AgentChatPanel from '../AgentChatPanel';

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

const selectChat = vi.hoisted(() => vi.fn());
const chatsApi = vi.hoisted(() => ({
  current: {
    sessionsState: { status: 'ok' },
    messagesState: { status: 'ok' },
    selectedId: 'master',
    selected: { id: 'master', title: 'Master', messages: [] },
    status: null,
    masterError: null,
    selectChat,
    refresh: vi.fn(),
    sendMessage: vi.fn().mockResolvedValue(true),
  },
}));
vi.mock('../../../lib/orchestration/useOrchestrationChats', () => ({
  MASTER_CHAT_KEY: 'master',
  SUBCONSCIOUS_CHAT_KEY: 'subconscious',
  useOrchestrationChats: () => chatsApi.current,
}));

// Surface the props we care about from the focus pane stub, and expose controls
// that invoke the container's composer + steering-review handlers.
interface FocusStubProps {
  canCompose: boolean;
  selected?: { id: string };
  composerBody: string;
  onComposerChange: (v: string) => void;
  onSubmitComposer: (e: { preventDefault: () => void }) => void;
  onRunSteeringReview: () => void;
}
vi.mock('../../intelligence/OrchestrationFocusPane', () => ({
  default: ({
    canCompose,
    selected,
    composerBody,
    onComposerChange,
    onSubmitComposer,
    onRunSteeringReview,
  }: FocusStubProps) => (
    <div
      data-testid="focus-pane"
      data-can-compose={String(canCompose)}
      data-selected={selected?.id}>
      <form data-testid="focus-form" onSubmit={onSubmitComposer}>
        <input
          data-testid="focus-input"
          value={composerBody}
          onChange={e => onComposerChange(e.target.value)}
        />
      </form>
      <button data-testid="focus-steering" onClick={() => onRunSteeringReview()}>
        review
      </button>
    </div>
  ),
}));

const subconsciousTrigger = vi.hoisted(() => vi.fn().mockResolvedValue(undefined));
vi.mock('../../../utils/tauriCommands/subconscious', () => ({ subconsciousTrigger }));

describe('AgentChatPanel', () => {
  beforeEach(() => vi.clearAllMocks());

  it('renders the master/subconscious toggle and the focus pane', () => {
    render(<AgentChatPanel />);
    expect(screen.getByTestId('orch-agent-tab-master')).toBeInTheDocument();
    expect(screen.getByTestId('orch-agent-tab-subconscious')).toBeInTheDocument();
    expect(screen.getByTestId('focus-pane')).toHaveAttribute('data-can-compose', 'true');
  });

  it('switches to the subconscious chat when its tab is clicked', () => {
    render(<AgentChatPanel />);
    fireEvent.click(screen.getByTestId('orch-agent-tab-subconscious'));
    expect(selectChat).toHaveBeenCalledWith('subconscious');
  });

  it('disables composing when the subconscious chat is selected', () => {
    chatsApi.current = { ...chatsApi.current, selectedId: 'subconscious', selectChat };
    render(<AgentChatPanel />);
    expect(screen.getByTestId('focus-pane')).toHaveAttribute('data-can-compose', 'false');
  });

  it('sends a composed message through the chats api', async () => {
    const sendMessage = vi.fn().mockResolvedValue(true);
    chatsApi.current = { ...chatsApi.current, selectedId: 'master', selectChat, sendMessage };
    render(<AgentChatPanel />);
    fireEvent.change(screen.getByTestId('focus-input'), { target: { value: 'hello' } });
    fireEvent.submit(screen.getByTestId('focus-form'));
    await waitFor(() => expect(sendMessage).toHaveBeenCalledWith(expect.anything(), 'hello'));
  });

  it('triggers a subconscious steering review', async () => {
    chatsApi.current = { ...chatsApi.current, selectChat };
    render(<AgentChatPanel />);
    fireEvent.click(screen.getByTestId('focus-steering'));
    await waitFor(() => expect(subconsciousTrigger).toHaveBeenCalledWith('tinyplace'));
  });
});
