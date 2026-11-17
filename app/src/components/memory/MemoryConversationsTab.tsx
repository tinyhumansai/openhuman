/**
 * Memory → Conversations: whether every turn is logged to memory
 * (`memory_policy_set { log_conversations }`), the agents whose turns are
 * logged (`memory_agents_list`) — opening one lists its stored conversation
 * items (`memory_items_list` filtered to `kinds: ["conversation"]` and the
 * agent) — and the consent-gated sync of chats from before logging was on.
 *
 * debug logging: DEBUG=openhuman:memory:conversations
 */
import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import {
  type Hit,
  type MemoryAgent,
  memoryAgentsList,
  memoryErrorMessage,
  memoryItemsList,
  type MemoryPolicy,
  memoryPolicyGet,
  memoryPolicySet,
} from '../../services/api/memoryApi';
import { Button, Card, Switch } from '../ui';
import { CenteredLoadingState } from '../ui/LoadingState';
import MemoryConversationsBackfill from './MemoryConversationsBackfill';
import MemoryErrorAlert from './MemoryErrorAlert';
import { fill } from './memoryFormat';
import MemoryHitRow from './MemoryHitRow';

const log = debug('openhuman:memory:conversations');

const PAGE_SIZE = 20;

export default function MemoryConversationsTab() {
  const { t } = useT();
  const [policy, setPolicy] = useState<MemoryPolicy | null>(null);
  const [agents, setAgents] = useState<MemoryAgent[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [openAgent, setOpenAgent] = useState<string | null>(null);
  const [items, setItems] = useState<Hit[] | null>(null);
  const [cursor, setCursor] = useState<string | null>(null);
  const [loadingItems, setLoadingItems] = useState(false);

  useEffect(() => {
    let cancelled = false;
    Promise.allSettled([memoryPolicyGet(), memoryAgentsList()]).then(([got, list]) => {
      if (cancelled) return;
      if (got.status === 'fulfilled') {
        setPolicy(got.value);
      } else {
        log('policy_get failed: %o', got.reason);
        setError(memoryErrorMessage(got.reason, t));
      }
      if (list.status === 'fulfilled') {
        log('agents: %d', list.value.agents?.length ?? 0);
        setAgents(list.value.agents ?? []);
      } else {
        log('agents_list failed: %o', list.reason);
        setError(prev => prev ?? memoryErrorMessage(list.reason, t));
        setAgents([]);
      }
    });
    return () => {
      cancelled = true;
    };
  }, [t]);

  const setLogging = async (next: boolean) => {
    setSaving(true);
    setError(null);
    try {
      log('set log_conversations=%s', next);
      setPolicy(await memoryPolicySet({ log_conversations: next }));
    } catch (err) {
      log('policy_set failed: %o', err);
      setError(memoryErrorMessage(err, t));
    } finally {
      setSaving(false);
    }
  };

  const loadItems = useCallback(
    async (agentId: string, after: string | null) => {
      setLoadingItems(true);
      try {
        const page = await memoryItemsList({
          filter: { kinds: ['conversation'], agent_id: agentId },
          limit: PAGE_SIZE,
          cursor: after ?? undefined,
        });
        log('items: agent=%s n=%d more=%s', agentId, page.items?.length ?? 0, !!page.next_cursor);
        setItems(prev => [...(after ? (prev ?? []) : []), ...(page.items ?? [])]);
        setCursor(page.next_cursor ?? null);
      } catch (err) {
        log('items_list failed: %o', err);
        setError(memoryErrorMessage(err, t));
        setItems(prev => prev ?? []);
      } finally {
        setLoadingItems(false);
      }
    },
    [t]
  );

  const toggleAgent = (agentId: string) => {
    if (openAgent === agentId) {
      setOpenAgent(null);
      return;
    }
    setOpenAgent(agentId);
    setItems(null);
    setCursor(null);
    void loadItems(agentId, null);
  };

  if (policy === null && agents === null) {
    return <CenteredLoadingState label={t('memoryPage.loading')} />;
  }

  return (
    <div className="space-y-4 animate-fade-up" data-testid="memory-conversations-tab">
      {error !== null && (
        <MemoryErrorAlert message={error} data-testid="memory-conversations-error" />
      )}

      {policy !== null && (
        <Card
          title={t('memoryPage.conversations.logTitle')}
          description={t('memoryPage.conversations.logDescription')}>
          <div className="flex items-center justify-between gap-4 px-4 py-3">
            <label htmlFor="memory-conversations-log" className="text-sm text-content">
              {t('memoryPage.conversations.logTurns')}
            </label>
            <Switch
              id="memory-conversations-log"
              data-testid="memory-conversations-log"
              checked={policy.log_conversations}
              disabled={saving}
              aria-label={t('memoryPage.conversations.logTurns')}
              onCheckedChange={next => void setLogging(next)}
            />
          </div>
        </Card>
      )}

      <MemoryConversationsBackfill />

      <Card
        title={t('memoryPage.conversations.agentsTitle')}
        description={t('memoryPage.conversations.agentsDescription')}
        data-testid="memory-conversations-agents">
        {(agents ?? []).length === 0 ? (
          <p
            className="px-4 py-3 text-sm text-content-muted"
            data-testid="memory-conversations-empty">
            {t('memoryPage.conversations.agentsEmpty')}
          </p>
        ) : (
          <ul className="divide-y divide-line-subtle">
            {(agents ?? []).map(agent => (
              <li key={agent.agent_id} data-testid={`memory-agent-${agent.agent_id}`}>
                <button
                  type="button"
                  className="flex w-full items-center justify-between gap-3 px-4 py-2.5 text-left hover:bg-surface-hover"
                  aria-expanded={openAgent === agent.agent_id}
                  data-testid={`memory-agent-${agent.agent_id}-open`}
                  onClick={() => toggleAgent(agent.agent_id)}>
                  <span className="truncate font-mono text-xs text-content">{agent.agent_id}</span>
                  <span className="shrink-0 text-xs text-content-muted">
                    {fill(t('memoryPage.conversations.turns'), { count: agent.turns })}
                  </span>
                </button>
                {openAgent === agent.agent_id && (
                  <div
                    className="border-t border-line-subtle"
                    data-testid={`memory-agent-${agent.agent_id}-items`}>
                    {items === null ? (
                      <CenteredLoadingState label={t('memoryPage.loading')} />
                    ) : items.length === 0 ? (
                      <p className="px-4 py-3 text-sm text-content-muted">
                        {t('memoryPage.conversations.itemsEmpty')}
                      </p>
                    ) : (
                      <ul className="divide-y divide-line-subtle">
                        {items.map(item => (
                          <MemoryHitRow
                            key={item.id}
                            id={item.id}
                            kind={item.kind}
                            text={item.text}
                            meta={item.meta}
                          />
                        ))}
                      </ul>
                    )}
                    {cursor && (
                      <div className="px-4 py-3">
                        <Button
                          type="button"
                          variant="secondary"
                          size="sm"
                          data-testid="memory-conversations-more"
                          disabled={loadingItems}
                          onClick={() => void loadItems(agent.agent_id, cursor)}>
                          {t('memoryPage.loadMore')}
                        </Button>
                      </div>
                    )}
                  </div>
                )}
              </li>
            ))}
          </ul>
        )}
      </Card>
    </div>
  );
}
