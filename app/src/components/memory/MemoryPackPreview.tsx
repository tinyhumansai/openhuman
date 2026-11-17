/**
 * Memory → Ask → Pack preview: the token-budgeted memory pack injected before
 * a turn (`memory_pack_preview`). With a query it previews what that turn
 * would get; without, what a new session starts with. An optional agent
 * picker (fed by `memory_agents_list`) previews another agent's pack. Shows
 * the rendered markdown, its size against the recall budget
 * (`memory_policy_get`), each section's hit count and the sections skipped,
 * with why.
 *
 * debug logging: DEBUG=openhuman:memory:pack
 */
import debug from 'debug';
import { type FormEvent, useEffect, useState } from 'react';

import { BubbleMarkdown } from '../../features/conversations/components/AgentMessageBubble';
import { useT } from '../../lib/i18n/I18nContext';
import {
  type MemoryAgent,
  memoryAgentsList,
  memoryErrorMessage,
  memoryPackPreview,
  memoryPolicyGet,
  type PackPreview,
} from '../../services/api/memoryApi';
import { Badge, Button, Card, Label, NativeSelect, TextArea } from '../ui';
import { CenteredLoadingState } from '../ui/LoadingState';
import MemoryErrorAlert from './MemoryErrorAlert';
import { fill } from './memoryFormat';

const log = debug('openhuman:memory:pack');

export default function MemoryPackPreview() {
  const { t } = useT();
  const [query, setQuery] = useState('');
  const [agentId, setAgentId] = useState('');
  const [agents, setAgents] = useState<MemoryAgent[]>([]);
  const [budget, setBudget] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [preview, setPreview] = useState<PackPreview | null>(null);

  // The agent list and the budget only decorate the preview; either failing
  // leaves the form usable.
  useEffect(() => {
    let cancelled = false;
    Promise.allSettled([memoryAgentsList(), memoryPolicyGet()]).then(([list, policy]) => {
      if (cancelled) return;
      if (list.status === 'fulfilled') {
        log('agents: %d', list.value.agents?.length ?? 0);
        setAgents(list.value.agents ?? []);
      } else {
        log('agents_list failed: %o', list.reason);
      }
      if (policy.status === 'fulfilled') setBudget(policy.value.recall?.budget_tokens ?? null);
      else log('policy_get failed: %o', policy.reason);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  const submit = async (event?: FormEvent) => {
    event?.preventDefault();
    if (busy) return;
    const text = query.trim();
    setBusy(true);
    setError(null);
    try {
      log('preview: mode=%s agent=%s', text ? 'turn' : 'session', agentId || 'default');
      const res = await memoryPackPreview({
        query: text || undefined,
        agent_id: agentId || undefined,
      });
      setPreview(res);
    } catch (err) {
      log('preview failed: %o', err);
      setError(memoryErrorMessage(err, t));
    } finally {
      setBusy(false);
    }
  };

  const pack = preview?.pack;
  const sections = pack?.sections ?? [];
  const skipped = pack?.skipped ?? [];

  return (
    <div className="space-y-4" data-testid="memory-pack-preview">
      <Card padded divided={false}>
        <form className="flex flex-col gap-3" onSubmit={e => void submit(e)}>
          <Label htmlFor="memory-pack-query" className="text-xs text-content-secondary">
            {t('memoryPage.pack.queryLabel')}
          </Label>
          <TextArea
            id="memory-pack-query"
            data-testid="memory-pack-query"
            rows={2}
            value={query}
            placeholder={t('memoryPage.pack.queryPlaceholder')}
            onChange={e => setQuery(e.target.value)}
          />
          <p className="text-[11px] leading-4 text-content-muted">
            {t('memoryPage.pack.queryHelp')}
          </p>
          <div className="flex flex-wrap items-center justify-between gap-3">
            <NativeSelect
              aria-label={t('memoryPage.pack.agentLabel')}
              data-testid="memory-pack-agent"
              value={agentId}
              onChange={e => setAgentId(e.target.value)}>
              <option value="">{t('memoryPage.pack.agentDefault')}</option>
              {agents.map(agent => (
                <option key={agent.agent_id} value={agent.agent_id}>
                  {agent.agent_id}
                </option>
              ))}
            </NativeSelect>
            <Button
              type="submit"
              variant="primary"
              size="sm"
              analyticsId="memory-pack-preview"
              data-testid="memory-pack-submit"
              disabled={busy}>
              {t('memoryPage.pack.preview')}
            </Button>
          </div>
        </form>
      </Card>

      {error !== null && <MemoryErrorAlert message={error} data-testid="memory-pack-error" />}

      {busy && <CenteredLoadingState label={t('memoryPage.pack.building')} />}

      {!busy && preview && pack && (
        <Card
          title={
            preview.mode === 'session'
              ? t('memoryPage.pack.sessionTitle')
              : t('memoryPage.pack.turnTitle')
          }
          description={fill(t('memoryPage.pack.scope'), {
            agent: preview.agent_id,
            root: preview.root,
            engine: pack.engine,
          })}
          headerRight={
            <Badge
              variant={budget !== null && pack.tokens > budget ? 'warning' : 'neutral'}
              data-testid="memory-pack-tokens">
              {budget !== null
                ? fill(t('memoryPage.pack.tokensOfBudget'), { tokens: pack.tokens, budget })
                : fill(t('memoryPage.pack.tokens'), { tokens: pack.tokens })}
            </Badge>
          }
          data-testid="memory-pack-result">
          <div className="px-4 py-3" data-testid="memory-pack-markdown">
            {pack.markdown.trim() ? (
              <BubbleMarkdown content={pack.markdown} />
            ) : (
              <p className="text-sm text-content-muted">{t('memoryPage.pack.empty')}</p>
            )}
          </div>
          {sections.length > 0 && (
            <div>
              <h4 className="px-4 pt-3 text-[10px] font-semibold uppercase tracking-wide text-content-faint">
                {t('memoryPage.pack.sections')}
              </h4>
              <ul className="divide-y divide-line-subtle" data-testid="memory-pack-sections">
                {sections.map(section => (
                  <li
                    key={section.heading}
                    className="flex items-center justify-between gap-3 px-4 py-2.5 text-sm"
                    data-testid={`memory-pack-section-${section.heading}`}>
                    <span className="truncate text-content">{section.heading}</span>
                    <span className="shrink-0 text-xs text-content-muted">
                      {fill(t('memoryPage.pack.hits'), { count: section.hits?.length ?? 0 })}
                    </span>
                  </li>
                ))}
              </ul>
            </div>
          )}
          {skipped.length > 0 && (
            <div>
              <h4 className="px-4 pt-3 text-[10px] font-semibold uppercase tracking-wide text-content-faint">
                {t('memoryPage.pack.skipped')}
              </h4>
              <ul className="divide-y divide-line-subtle" data-testid="memory-pack-skipped">
                {skipped.map(entry => (
                  <li
                    key={entry.heading}
                    className="flex items-start justify-between gap-3 px-4 py-2.5 text-sm"
                    data-testid={`memory-pack-skipped-${entry.heading}`}>
                    <span className="truncate text-content">{entry.heading}</span>
                    <span className="text-right text-xs text-content-muted">{entry.reason}</span>
                  </li>
                ))}
              </ul>
            </div>
          )}
        </Card>
      )}
    </div>
  );
}
