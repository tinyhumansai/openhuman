/**
 * AgentChatPanel — "chat with the main agent and see its subconscious".
 *
 * The focused conversation surface of the Orchestration tab: a two-way toggle
 * between the Master chat (you ↔ the main agent) and the Subconscious chat (the
 * background steering loop), rendered through the shared {@link OrchestrationFocusPane}.
 * Connections/discovery moved to their own sub-pages, so this panel deliberately
 * drops the contact rail and keeps just the agent dialogue + steering controls.
 */
import debugFactory from 'debug';
import { type FormEvent, useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import {
  MASTER_CHAT_KEY,
  SUBCONSCIOUS_CHAT_KEY,
  useOrchestrationChats,
} from '../../lib/orchestration/useOrchestrationChats';
import { subconsciousTrigger } from '../../utils/tauriCommands/subconscious';
import OrchestrationFocusPane from '../intelligence/OrchestrationFocusPane';

const debug = debugFactory('orchestration:agent-chat');

export default function AgentChatPanel() {
  const { t } = useT();
  const {
    sessionsState,
    messagesState,
    selectedId,
    selected,
    status,
    masterError,
    selectChat,
    refresh,
    sendMessage,
  } = useOrchestrationChats(t);

  const [composerBody, setComposerBody] = useState('');
  const [sending, setSending] = useState(false);
  const [runningReview, setRunningReview] = useState(false);
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const submitComposer = useCallback(
    (event: FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      const body = composerBody.trim();
      if (!body || sending) return;
      setSending(true);
      void sendMessage(selected, body).then(ok => {
        if (!mountedRef.current) return;
        if (ok) setComposerBody('');
        setSending(false);
      });
    },
    [composerBody, sending, sendMessage, selected]
  );

  const runSteeringReview = useCallback(async () => {
    debug('steering review: trigger');
    setRunningReview(true);
    try {
      await subconsciousTrigger('tinyplace');
    } catch (err) {
      debug('steering review trigger failed: %o', err);
    } finally {
      setRunningReview(false);
    }
  }, []);

  const steeringText = status?.steering?.text?.trim() || null;
  const isMasterSelected = selectedId === MASTER_CHAT_KEY;

  const tabs: Array<{ key: string; label: string }> = [
    { key: MASTER_CHAT_KEY, label: t('orchPage.agent.mainTab') },
    { key: SUBCONSCIOUS_CHAT_KEY, label: t('orchPage.agent.subconsciousTab') },
  ];

  return (
    <div className="flex h-full min-h-[520px] flex-col overflow-hidden rounded-xl border border-line bg-surface shadow-soft">
      {/* Master ↔ Subconscious switch. */}
      <div
        className="flex shrink-0 items-center gap-1 border-b border-line px-3 py-2"
        role="tablist"
        aria-label={t('orchPage.agent.description')}>
        {tabs.map(tab => {
          const active = selectedId === tab.key;
          return (
            <button
              key={tab.key}
              type="button"
              role="tab"
              aria-selected={active}
              data-testid={`orch-agent-tab-${tab.key}`}
              onClick={() => selectChat(tab.key)}
              className={`rounded-md px-3 py-1.5 text-sm transition-colors ${
                active
                  ? 'bg-surface-subtle font-medium text-content'
                  : 'text-content-secondary hover:bg-surface-hover hover:text-content'
              }`}>
              {tab.label}
            </button>
          );
        })}
      </div>

      <div className="flex min-h-0 flex-1 flex-col">
        <OrchestrationFocusPane
          selected={selected}
          sessionsState={sessionsState}
          messagesState={messagesState}
          status={status}
          masterError={masterError}
          refresh={refresh}
          steeringText={steeringText}
          runningReview={runningReview}
          onRunSteeringReview={() => void runSteeringReview()}
          canCompose={isMasterSelected}
          composerBody={composerBody}
          onComposerChange={setComposerBody}
          sending={sending}
          onSubmitComposer={submitComposer}
        />
      </div>
    </div>
  );
}
