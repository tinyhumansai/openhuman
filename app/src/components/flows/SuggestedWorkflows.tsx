/**
 * SuggestedWorkflows — the "Suggested for you" section on the Flows page.
 *
 * Surfaces the read-only Flow Scout's workflow suggestions as friendly cards.
 * A "Discover" button runs the `flow_discovery` agent
 * (`openhuman.flows_discover`), which reasons over the user's
 * memory/threads/connections/existing flows and records concrete, buildable
 * suggestions. Each card shows the pitch (title, one-liner, rationale) plus two
 * actions:
 *
 *   - "Build this" hands the suggestion's `build_prompt` to the existing
 *     `workflow_builder` agent (via {@link useWorkflowBuilderChat}), rendering
 *     the returned {@link WorkflowProposalCard} inline. Saving from that card
 *     marks the suggestion `built` (drops it from the active list).
 *   - "Dismiss" marks the suggestion `dismissed` (kept server-side so a later
 *     discovery run won't re-surface it).
 *
 * Nothing here persists or enables a flow directly — discovery is read-only and
 * saving stays behind the proposal card's explicit "Save & enable" click.
 */
import createDebug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { useWorkflowBuilderChat } from '../../hooks/useWorkflowBuilderChat';
import { useT } from '../../lib/i18n/I18nContext';
import {
  discoverWorkflows,
  dismissSuggestion,
  type FlowSuggestion,
  listSuggestions,
  markSuggestionBuilt,
} from '../../services/api/flowsApi';
import WorkflowProposalCard from '../chat/WorkflowProposalCard';
import Button from '../ui/Button';

const log = createDebug('app:flows:suggested');

/** Maps a `trigger_hint` to a short, translated badge label. */
function triggerLabelKey(hint?: string | null): string | null {
  switch (hint) {
    case 'schedule':
      return 'flows.suggest.trigger.schedule';
    case 'app_event':
      return 'flows.suggest.trigger.app_event';
    case 'manual':
      return 'flows.suggest.trigger.manual';
    default:
      return null;
  }
}

interface SuggestionCardProps {
  suggestion: FlowSuggestion;
  building: boolean;
  onBuild: () => void;
  onDismiss: () => void;
}

function SuggestionCard({ suggestion, building, onBuild, onDismiss }: SuggestionCardProps) {
  const { t } = useT();
  const triggerKey = triggerLabelKey(suggestion.trigger_hint);

  return (
    <div
      data-testid="flow-suggestion-card"
      className="rounded-xl border border-line bg-surface p-3 text-sm">
      <div className="flex items-start justify-between gap-2">
        <p className="font-semibold text-content">{suggestion.title}</p>
        {triggerKey && (
          <span className="shrink-0 rounded-full bg-ocean-50 px-2 py-0.5 text-xs text-ocean-700 dark:bg-ocean-500/10 dark:text-ocean-200">
            {t(triggerKey)}
          </span>
        )}
      </div>
      <p className="mt-1 text-content-secondary">{suggestion.one_liner}</p>
      <p className="mt-2 text-xs text-content-muted">
        <span className="font-medium">{t('flows.suggest.why')}:</span> {suggestion.rationale}
      </p>

      {suggestion.suggested_connections.length > 0 && (
        <p className="mt-1 text-xs text-content-faint">
          {t('flows.suggest.uses')}: {suggestion.suggested_connections.join(', ')}
        </p>
      )}

      <div className="mt-3 flex items-center gap-2">
        <Button
          type="button"
          variant="primary"
          size="sm"
          data-testid="flow-suggestion-build"
          disabled={building}
          onClick={onBuild}>
          {building ? t('flows.suggest.building') : t('flows.suggest.build')}
        </Button>
        <Button
          type="button"
          variant="tertiary"
          size="sm"
          data-testid="flow-suggestion-dismiss"
          onClick={onDismiss}>
          {t('flows.suggest.dismiss')}
        </Button>
      </div>
    </div>
  );
}

export default function SuggestedWorkflows() {
  const { t } = useT();
  const [suggestions, setSuggestions] = useState<FlowSuggestion[]>([]);
  const [discovering, setDiscovering] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** The suggestion currently being authored inline, or `null`. */
  const [buildingId, setBuildingId] = useState<string | null>(null);

  const { threadId, sending, proposal, send } = useWorkflowBuilderChat();

  // Load any previously-discovered active suggestions on mount.
  useEffect(() => {
    let cancelled = false;
    void listSuggestions('new')
      .then(loaded => {
        if (!cancelled) setSuggestions(loaded);
      })
      .catch(e => log('initial listSuggestions failed: %o', e));
    return () => {
      cancelled = true;
    };
  }, []);

  const discover = useCallback(async () => {
    if (discovering) return;
    setDiscovering(true);
    setError(null);
    try {
      const fresh = await discoverWorkflows();
      setSuggestions(fresh);
    } catch (e) {
      log('discoverWorkflows failed: %o', e);
      setError(t('flows.suggest.error'));
    } finally {
      setDiscovering(false);
    }
  }, [discovering, t]);

  const removeSuggestion = useCallback((id: string) => {
    setSuggestions(prev => prev.filter(s => s.id !== id));
  }, []);

  const onBuild = useCallback(
    async (suggestion: FlowSuggestion) => {
      if (sending) return;
      setBuildingId(suggestion.id);
      await send({
        displayText: suggestion.title,
        request: { mode: 'create', instruction: suggestion.build_prompt },
      });
    },
    [sending, send]
  );

  const onDismiss = useCallback(
    async (id: string) => {
      // Optimistically remove; reconcile on failure by reloading.
      removeSuggestion(id);
      try {
        await dismissSuggestion(id);
      } catch (e) {
        log('dismissSuggestion failed: %o', e);
        void listSuggestions('new')
          .then(setSuggestions)
          .catch(() => {});
      }
    },
    [removeSuggestion]
  );

  const onSaved = useCallback(
    (id: string) => {
      removeSuggestion(id);
      setBuildingId(null);
      void markSuggestionBuilt(id).catch(e => log('markSuggestionBuilt failed: %o', e));
    },
    [removeSuggestion]
  );

  const hasSuggestions = suggestions.length > 0;

  return (
    <section
      data-testid="suggested-workflows"
      className="rounded-xl border border-line bg-surface/50 p-3">
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <h3 className="flex items-center gap-1.5 text-sm font-semibold text-content">
            <span aria-hidden>✨</span>
            {t('flows.suggest.title')}
          </h3>
          <p className="text-xs text-content-muted">{t('flows.suggest.subtitle')}</p>
        </div>
        <Button
          type="button"
          variant="secondary"
          size="sm"
          data-testid="flow-suggestions-discover"
          disabled={discovering}
          onClick={() => void discover()}>
          {discovering
            ? t('flows.suggest.discovering')
            : hasSuggestions
              ? t('flows.suggest.rediscover')
              : t('flows.suggest.discover')}
        </Button>
      </div>

      {error && (
        <p className="mt-2 text-xs text-coral" data-testid="flow-suggestions-error">
          {error}
        </p>
      )}

      {!hasSuggestions && !discovering && (
        <p className="mt-3 text-xs text-content-faint" data-testid="flow-suggestions-empty">
          {t('flows.suggest.empty')}
        </p>
      )}

      {hasSuggestions && (
        <div className="mt-3 grid gap-2 sm:grid-cols-2">
          {suggestions.map(suggestion => (
            <SuggestionCard
              key={suggestion.id}
              suggestion={suggestion}
              building={buildingId === suggestion.id && sending}
              onBuild={() => void onBuild(suggestion)}
              onDismiss={() => void onDismiss(suggestion.id)}
            />
          ))}
        </div>
      )}

      {/* The workflow_builder proposal, rendered inline once a "Build this" turn
          returns. Saving from the card marks the originating suggestion built. */}
      {threadId && proposal && buildingId && (
        <div className="mt-3" data-testid="flow-suggestion-proposal">
          <WorkflowProposalCard
            threadId={threadId}
            proposal={proposal}
            onSaved={() => onSaved(buildingId)}
          />
        </div>
      )}
    </section>
  );
}
