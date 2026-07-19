import { fireEvent, render, screen, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { apiClient } from '../../agentworld/AgentWorldShell';
import type { ContactView } from '../../lib/agentworld/invokeApiClient';
import OrchestrationSidebar, { type OrchestrationSidebarProps } from './OrchestrationSidebar';

vi.mock('../../agentworld/AgentWorldShell', () => ({
  apiClient: {
    orchestrationPairing: {
      acceptRequest: vi.fn(async () => ({})),
      declineRequest: vi.fn(async () => ({})),
      blockRequest: vi.fn(async () => ({})),
    },
  },
}));

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

const acceptMock = vi.mocked(apiClient.orchestrationPairing.acceptRequest);
const declineMock = vi.mocked(apiClient.orchestrationPairing.declineRequest);
const blockMock = vi.mocked(apiClient.orchestrationPairing.blockRequest);

const REQUESTER = '3icjiLXhn6BMv43MsHjpKKxm7hEYBk7R5rvNXB1HUk7g';

const request = (): ContactView =>
  ({ agentId: REQUESTER, status: 'pending', direction: 'incoming' }) as ContactView;

let onCreateSession: ReturnType<typeof vi.fn>;
let onToggleContact: ReturnType<typeof vi.fn>;

const props = (over: Partial<OrchestrationSidebarProps>): OrchestrationSidebarProps =>
  ({
    relayInfo: null,
    onRefreshAll: vi.fn(),
    refreshDisabled: false,
    steeringText: null,
    selfIdentity: null,
    identityLoading: false,
    onPublishIdentity: vi.fn(),
    publishingIdentity: false,
    publishIdentityError: null,
    attentionQueue: null,
    attentionLoading: false,
    onAttentionAction: vi.fn(),
    linkAgentId: '',
    onLinkAgentIdChange: vi.fn(),
    onSubmitLink: vi.fn(),
    pairingAction: null,
    contactStats: null,
    incomingRequests: [],
    outgoingCount: 0,
    pairingError: null,
    agentHandles: {},
    // Invoke the thunk so the underlying apiClient call is exercised.
    runPairingAction: vi.fn((_id: string, thunk: () => Promise<unknown>) => thunk()),
    pinned: [],
    selectedId: null,
    onSelectChat: vi.fn(),
    acceptedContactList: [],
    expandedContacts: {},
    onToggleContact,
    sessionsByContact: new Map(),
    creatingSession: null,
    onCreateSession,
    acceptedContacts: new Set<string>(),
    pendingContacts: new Set<string>(),
    ungroupedSessions: [],
    ...over,
  }) as OrchestrationSidebarProps;

describe('OrchestrationSidebar', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    onCreateSession = vi.fn();
    onToggleContact = vi.fn();
  });

  it('runs accept / decline / block on an incoming request, resolving its address', () => {
    render(
      <OrchestrationSidebar
        {...props({ incomingRequests: [request()], agentHandles: { [REQUESTER]: 'peer' } })}
      />
    );

    // Handle is shown additively alongside the raw address.
    expect(screen.getByText('@peer')).toBeInTheDocument();
    expect(screen.getByText(REQUESTER)).toBeInTheDocument();

    fireEvent.click(screen.getByText('tinyplaceOrchestration.pairing.accept'));
    fireEvent.click(screen.getByText('tinyplaceOrchestration.pairing.decline'));
    fireEvent.click(screen.getByText('tinyplaceOrchestration.pairing.block'));

    expect(acceptMock).toHaveBeenCalledWith(REQUESTER);
    expect(declineMock).toHaveBeenCalledWith(REQUESTER);
    expect(blockMock).toHaveBeenCalledWith(REQUESTER);
  });

  it('expands a contact and starts a new session under it', () => {
    const contact = {
      agentId: REQUESTER,
      status: 'accepted',
      direction: 'incoming',
    } as ContactView;

    render(
      <OrchestrationSidebar
        {...props({
          acceptedContactList: [contact],
          expandedContacts: { [REQUESTER]: true },
          agentHandles: { [REQUESTER]: 'peer' },
        })}
      />
    );

    fireEvent.click(screen.getByTestId(`tinyplace-new-session-${REQUESTER}`));
    expect(onCreateSession).toHaveBeenCalledWith(REQUESTER);

    fireEvent.click(
      within(screen.getByTestId(`tinyplace-contact-${REQUESTER}`)).getByText('@peer')
    );
    expect(onToggleContact).toHaveBeenCalledWith(REQUESTER);
  });
});
