/**
 * OrchestrationSidebar — the left rail of the TinyPlace Orchestration tab: the
 * topbar (title + relay badge + refresh + launch shell), the self-identity card,
 * the "Needs you" attention zone, the pairing panel (link form, stats, incoming
 * requests), and the roster tree (pinned chats + accepted contacts with their
 * nested sessions + ungrouped sessions).
 *
 * Presentational: all state + handlers come from the tab container. It imports
 * `apiClient` and the shared helpers directly so the request-action JSX stays
 * identical to the pre-extraction container.
 */
import debugFactory from 'debug';
import type { FormEvent, ReactElement } from 'react';

import { apiClient } from '../../agentworld/AgentWorldShell';
import type { ContactView, PairingSnapshot } from '../../lib/agentworld/invokeApiClient';
import { useT } from '../../lib/i18n/I18nContext';
import type {
  AttentionAction,
  AttentionQueue,
  RelayInfo,
  SelfIdentity,
} from '../../lib/orchestration/orchestrationClient';
import type { ChatWindow } from '../../lib/orchestration/useOrchestrationChats';
import Button from '../ui/Button';
import AttentionQueueView from './AttentionQueue';
import { ChatListButton } from './OrchestrationChatPrimitives';
import { contactAddress, contactBadgeKey, truncate } from './orchestrationTabHelpers';
import RelayBadge from './RelayBadge';
import SelfIdentityCard from './SelfIdentityCard';

const debug = debugFactory('brain:tinyplace-orchestration');

export interface OrchestrationSidebarProps {
  relayInfo: RelayInfo | null;
  onRefreshAll: () => void;
  refreshDisabled: boolean;
  steeringText: string | null;
  selfIdentity: SelfIdentity | null;
  identityLoading: boolean;
  onPublishIdentity: () => void;
  publishingIdentity: boolean;
  publishIdentityError: string | null;
  attentionQueue: AttentionQueue | null;
  attentionLoading: boolean;
  onAttentionAction: (action: AttentionAction) => void;
  linkAgentId: string;
  onLinkAgentIdChange: (value: string) => void;
  onSubmitLink: (event: FormEvent<HTMLFormElement>) => void;
  pairingAction: string | null;
  contactStats: PairingSnapshot['stats'] | null;
  incomingRequests: ContactView[];
  outgoingCount: number;
  pairingError: string | null;
  agentHandles: Record<string, string | null>;
  runPairingAction: (actionId: string, action: () => Promise<unknown>) => Promise<void>;
  pinned: ChatWindow[];
  selectedId: string | null;
  onSelectChat: (id: string) => void;
  acceptedContactList: ContactView[];
  expandedContacts: Record<string, boolean>;
  onToggleContact: (address: string) => void;
  sessionsByContact: Map<string, ChatWindow[]>;
  creatingSession: string | null;
  onCreateSession: (address: string) => void;
  acceptedContacts: Set<string>;
  pendingContacts: Set<string>;
  ungroupedSessions: ChatWindow[];
}

export default function OrchestrationSidebar({
  relayInfo,
  onRefreshAll,
  refreshDisabled,
  steeringText,
  selfIdentity,
  identityLoading,
  onPublishIdentity,
  publishingIdentity,
  publishIdentityError,
  attentionQueue,
  attentionLoading,
  onAttentionAction,
  linkAgentId,
  onLinkAgentIdChange,
  onSubmitLink,
  pairingAction,
  contactStats,
  incomingRequests,
  outgoingCount,
  pairingError,
  agentHandles,
  runPairingAction,
  pinned,
  selectedId,
  onSelectChat,
  acceptedContactList,
  expandedContacts,
  onToggleContact,
  sessionsByContact,
  creatingSession,
  onCreateSession,
  acceptedContacts,
  pendingContacts,
  ungroupedSessions,
}: OrchestrationSidebarProps): ReactElement {
  const { t } = useT();
  return (
    <aside className="flex w-80 flex-none flex-col border-r border-line bg-surface-muted/40">
      <div className="border-b border-line px-4 py-3">
        <div className="flex items-center justify-between gap-3">
          <div className="min-w-0">
            <div className="flex items-center gap-1.5">
              <h3 className="truncate text-sm font-semibold text-content">
                {t('tinyplaceOrchestration.title')}
              </h3>
              <RelayBadge relay={relayInfo} />
            </div>
            <p className="mt-0.5 truncate text-[11px] text-content-muted">
              {t('tinyplaceOrchestration.subtitle')}
            </p>
          </div>
          <div className="flex flex-none items-center gap-1.5">
            <Button
              variant="secondary"
              size="sm"
              onClick={onRefreshAll}
              aria-label={t('tinyplaceOrchestration.refresh')}
              disabled={refreshDisabled}>
              {t('tinyplaceOrchestration.refresh')}
            </Button>
            {/* Launch shell — external instance spawn is wired in a later PR. */}
            <Button
              variant="primary"
              size="sm"
              data-testid="tinyplace-new-instance"
              disabled
              title={t('tinyplaceOrchestration.newInstanceSoon')}>
              {t('tinyplaceOrchestration.newInstance')}
            </Button>
          </div>
        </div>
        {steeringText ? (
          <div
            data-testid="tinyplace-steering-chip"
            className="mt-2 flex items-start gap-1.5 rounded-md bg-amber-50 px-2 py-1 text-[11px] text-amber-700 dark:bg-amber-500/10 dark:text-amber-300">
            <span className="flex-none font-semibold uppercase tracking-wide">
              {t('tinyplaceOrchestration.steering.label')}
            </span>
            <span className="min-w-0 flex-1 truncate">{truncate(steeringText, 72)}</span>
          </div>
        ) : null}
      </div>

      <SelfIdentityCard
        identity={selfIdentity}
        loading={identityLoading}
        onPublish={onPublishIdentity}
        publishing={publishingIdentity}
        publishError={publishIdentityError}
      />

      <AttentionQueueView
        queue={attentionQueue}
        loading={attentionLoading}
        onAction={onAttentionAction}
      />

      <section className="border-b border-line px-4 py-3">
        <form className="space-y-2" onSubmit={onSubmitLink}>
          <label
            htmlFor="tinyplace-session-agent-id"
            className="block text-[10px] font-semibold uppercase tracking-wide text-content-muted">
            {t('tinyplaceOrchestration.pairing.linkLabel')}
          </label>
          <div className="flex gap-2">
            <input
              id="tinyplace-session-agent-id"
              value={linkAgentId}
              onChange={event => onLinkAgentIdChange(event.target.value)}
              placeholder={t('tinyplaceOrchestration.pairing.linkPlaceholder')}
              className="min-w-0 flex-1 rounded-md border border-line bg-surface px-2 py-1.5 text-xs text-content outline-none transition focus:border-ocean-500 focus:ring-2 focus:ring-ocean-500/20"
            />
            <Button
              type="submit"
              variant="secondary"
              size="sm"
              disabled={!linkAgentId.trim() || pairingAction !== null}>
              {t('tinyplaceOrchestration.pairing.linkAction')}
            </Button>
          </div>
        </form>

        <div className="mt-2 flex flex-wrap gap-1.5 text-[10px] text-content-faint">
          <span className="rounded-full bg-surface-strong px-2 py-0.5">
            {t('tinyplaceOrchestration.pairing.linked')}: {contactStats?.contactCount ?? 0}
          </span>
          <span className="rounded-full bg-surface-strong px-2 py-0.5">
            {t('tinyplaceOrchestration.pairing.incoming')}: {incomingRequests.length}
          </span>
          <span className="rounded-full bg-surface-strong px-2 py-0.5">
            {t('tinyplaceOrchestration.pairing.outgoing')}: {outgoingCount}
          </span>
        </div>

        {pairingError ? (
          <p className="mt-2 rounded-md bg-coral-50 px-2 py-1 text-xs text-coral-700 dark:bg-coral-500/10 dark:text-coral-300">
            {pairingError}
          </p>
        ) : null}

        {incomingRequests.length > 0 ? (
          <div className="mt-3 space-y-2">
            <h4 className="text-[10px] font-semibold uppercase tracking-wide text-content-muted">
              {t('tinyplaceOrchestration.pairing.requests')}
            </h4>
            {incomingRequests.map((request, index) => {
              const address = contactAddress(request);
              const handle = address ? agentHandles[address] : null;
              return (
                <div
                  key={address || `request-${index}`}
                  className="rounded-lg border border-line bg-surface px-2 py-2">
                  {handle ? (
                    <div className="truncate text-xs font-medium text-content">@{handle}</div>
                  ) : null}
                  <div className="truncate font-mono text-[11px] text-content-muted">{address}</div>
                  <div className="mt-2 flex gap-1.5">
                    <Button
                      variant="primary"
                      size="sm"
                      disabled={pairingAction !== null || !address}
                      onClick={() =>
                        void runPairingAction(`accept:${address}`, () =>
                          apiClient.orchestrationPairing.acceptRequest(address)
                        )
                      }>
                      {t('tinyplaceOrchestration.pairing.accept')}
                    </Button>
                    <Button
                      variant="secondary"
                      size="sm"
                      disabled={pairingAction !== null || !address}
                      onClick={() =>
                        void runPairingAction(`remove:${address}`, () =>
                          apiClient.orchestrationPairing.declineRequest(address)
                        )
                      }>
                      {t('tinyplaceOrchestration.pairing.decline')}
                    </Button>
                    <Button
                      variant="secondary"
                      size="sm"
                      disabled={pairingAction !== null || !address}
                      onClick={() =>
                        void runPairingAction(`block:${address}`, () =>
                          apiClient.orchestrationPairing.blockRequest(address)
                        )
                      }>
                      {t('tinyplaceOrchestration.pairing.block')}
                    </Button>
                  </div>
                </div>
              );
            })}
          </div>
        ) : null}
      </section>

      <div className="min-h-0 flex-1 overflow-y-auto">
        <section>
          <h4 className="px-3 pb-1 pt-3 text-[10px] font-semibold uppercase tracking-wide text-content-muted">
            {t('tinyplaceOrchestration.pinned')}
          </h4>
          <div>
            {pinned.map(chat => (
              <ChatListButton
                key={chat.id}
                chat={chat}
                selected={selectedId === chat.id}
                onSelect={() => {
                  debug('[tinyplace-orchestration] open pinned id=%s', chat.id);
                  onSelectChat(chat.id);
                }}
              />
            ))}
          </div>
        </section>

        <section>
          <h4 className="px-3 pb-1 pt-3 text-[10px] font-semibold uppercase tracking-wide text-content-muted">
            {t('tinyplaceOrchestration.contacts')}
          </h4>
          {acceptedContactList.length === 0 ? (
            <div className="px-4 py-8 text-center text-sm text-content-faint">
              {t('tinyplaceOrchestration.noContacts')}
            </div>
          ) : (
            <div className="space-y-1 px-2 pb-2">
              {acceptedContactList.map((contact, index) => {
                const address = contactAddress(contact);
                const handle = address ? agentHandles[address] : null;
                const isOpen = !!expandedContacts[address];
                const contactSessions = address ? (sessionsByContact.get(address) ?? []) : [];
                return (
                  <div
                    key={address || `contact-${index}`}
                    className="overflow-hidden rounded-lg border border-line bg-surface">
                    <button
                      type="button"
                      data-testid={`tinyplace-contact-${address}`}
                      aria-expanded={isOpen}
                      onClick={() => onToggleContact(address)}
                      className="flex w-full items-center gap-2 px-2 py-2 text-left transition hover:bg-surface-hover">
                      <span className="flex-none text-[10px] text-content-muted">
                        {isOpen ? '▾' : '▸'}
                      </span>
                      <span className="min-w-0 flex-1">
                        {handle ? (
                          <span className="block truncate text-xs font-medium text-content">
                            @{handle}
                          </span>
                        ) : null}
                        <span className="block truncate font-mono text-[11px] text-content-muted">
                          {address}
                        </span>
                      </span>
                      {contactSessions.length > 0 ? (
                        <span className="flex-none rounded-full bg-surface-strong px-1.5 py-0.5 text-[10px] font-medium text-content-faint">
                          {contactSessions.length}
                        </span>
                      ) : null}
                    </button>
                    {isOpen ? (
                      <div className="border-t border-line-subtle">
                        {contactSessions.map(chat => (
                          <ChatListButton
                            key={chat.id}
                            chat={chat}
                            selected={selectedId === chat.id}
                            contactBadge={contactBadgeKey(chat, acceptedContacts, pendingContacts)}
                            onSelect={() => {
                              debug('[tinyplace-orchestration] open session id=%s', chat.id);
                              onSelectChat(chat.id);
                            }}
                          />
                        ))}
                        <button
                          type="button"
                          data-testid={`tinyplace-new-session-${address}`}
                          disabled={!address || creatingSession === address}
                          onClick={() => onCreateSession(address)}
                          className="flex w-full items-center gap-1 px-3 py-2 text-left text-[11px] font-medium text-ocean-500 transition hover:bg-surface-hover disabled:opacity-50">
                          + {t('tinyplaceOrchestration.newSession')}
                        </button>
                      </div>
                    ) : null}
                  </div>
                );
              })}
            </div>
          )}
        </section>

        {ungroupedSessions.length > 0 ? (
          <section>
            <h4 className="px-3 pb-1 pt-3 text-[10px] font-semibold uppercase tracking-wide text-content-muted">
              {t('tinyplaceOrchestration.otherSessions')}
            </h4>
            <div>
              {ungroupedSessions.map(chat => (
                <ChatListButton
                  key={chat.id}
                  chat={chat}
                  selected={selectedId === chat.id}
                  contactBadge={contactBadgeKey(chat, acceptedContacts, pendingContacts)}
                  onSelect={() => {
                    debug('[tinyplace-orchestration] open session id=%s', chat.id);
                    onSelectChat(chat.id);
                  }}
                />
              ))}
            </div>
          </section>
        ) : null}
      </div>
    </aside>
  );
}
