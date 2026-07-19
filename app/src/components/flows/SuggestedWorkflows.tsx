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
 *   - "Build this" creates a new blank flow (named from the suggestion's
 *     title, mirroring {@link WorkflowPromptBar}'s instant-create path), then
 *     navigates into the new flow's canvas with the suggestion's
 *     `build_prompt` PRE-FILLED into the copilot's input
 *     (`location.state.copilotPrefill`, carrying `mode: 'build'` so the
 *     first Send drives a full build → dry-run → propose turn against the
 *     just-created flow) — never auto-sent. The user reviews/edits the brief
 *     and presses Send themselves. The card is dropped from THIS session's
 *     local list right away (`removeSuggestion`), but `markSuggestionBuilt`
 *     is deliberately NOT called here: that RPC's contract is "the user
 *     SAVED a flow authored from this suggestion", and this path only
 *     creates a blank flow + an unsent prompt — the user may close the
 *     canvas, never press Send, reject the proposal, or navigate away
 *     without saving. There's no clean hook back from the canvas's Save to
 *     this suggestion id yet, so we leave it un-built server-side rather
 *     than risk permanently hiding an abandoned build from Flow Scout; it
 *     can simply resurface on a later discovery run.
 *   - "Dismiss" marks the suggestion `dismissed` (kept server-side so a later
 *     discovery run won't re-surface it).
 *
 * Nothing here persists or enables a flow directly beyond the blank-flow
 * create itself — the copilot only proposes, and the canvas's explicit Save
 * is the only thing that ever persists a built graph.
 */
import createDebug from 'debug';
import { useCallback, useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { createBlankWorkflowGraph, deriveWorkflowName } from '../../lib/flows/newFlow';
import { useT } from '../../lib/i18n/I18nContext';
import {
  createFlow,
  discoverWorkflows,
  dismissSuggestion,
  type FlowSuggestion,
  listSuggestions,
} from '../../services/api/flowsApi';
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
  /** True while THIS suggestion's blank flow is being created + navigated to. */
  opening: boolean;
  /**
   * True while ANY suggestion's blank flow is being created + navigated to —
   * disables every card's "Build this" (not just the active one) so a click
   * on a different card can't silently no-op against `onBuild`'s
   * `if (openingId) return` re-entry guard while a build is already in
   * flight.
   */
  buildInProgress: boolean;
  onBuild: () => void;
  onDismiss: () => void;
}

function SuggestionCard({
  suggestion,
  opening,
  buildInProgress,
  onBuild,
  onDismiss,
}: SuggestionCardProps) {
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
          disabled={buildInProgress}
          onClick={onBuild}>
          {opening ? t('flows.suggest.opening') : t('flows.suggest.build')}
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
  const navigate = useNavigate();
  const [suggestions, setSuggestions] = useState<FlowSuggestion[]>([]);
  const [discovering, setDiscovering] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** The suggestion whose blank flow is currently being created, or `null`. */
  const [openingId, setOpeningId] = useState<string | null>(null);

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

  // Mirrors `WorkflowPromptBar`'s instant-create path: creates a blank flow
  // named from the suggestion, then opens its canvas with the copilot's input
  // PRE-FILLED (never auto-sent) with the suggestion's `build_prompt`, tagged
  // `mode: 'build'` so the panel's first Send runs a full build → dry-run →
  // propose turn against this already-created (blank) flow — matching the
  // server's `BuildMode::Build` contract — rather than treating it as a
  // draft to merely `revise` (see `WorkflowCopilotPanel.submit`).
  //
  // Deliberately does NOT call `markSuggestionBuilt`: that RPC's contract is
  // "the user SAVED a flow authored from this suggestion" (the old inline
  // path only called it from the proposal card's "Save & enable" `onSaved`
  // callback). This path only creates a blank flow and pre-fills an unsent
  // prompt — the user may close the canvas, never press Send, reject the
  // copilot's proposal, or navigate away without saving, and marking built
  // here would permanently hide/dedupe a suggestion nothing was ever built
  // from. There's no clean hook yet from the canvas's Save back to the
  // originating suggestion id, so — per the safer option — we leave it
  // un-built server-side; it can simply reappear on a later discovery run.
  // We still drop it from THIS session's local list (`removeSuggestion`) so
  // it doesn't linger in the UI right after the user has already acted on
  // it once.
  const onBuild = useCallback(
    async (suggestion: FlowSuggestion) => {
      if (openingId) return;
      setOpeningId(suggestion.id);
      const name = deriveWorkflowName(suggestion.title, t('flows.page.newWorkflow'));
      try {
        log('onBuild: creating blank flow name=%s for suggestion=%s', name, suggestion.id);
        // Safe default: suggestion-authored flows require approval so outbound
        // Slack/Gmail/HTTP/code nodes cannot fire unattended, matching
        // `WorkflowPromptBar`'s instant-create default.
        const flow = await createFlow(
          name,
          createBlankWorkflowGraph(name, t('flows.nodeKind.trigger')),
          true
        );
        log('onBuild: created id=%s — opening canvas with prefill seed', flow.id);
        removeSuggestion(suggestion.id);
        navigate(`/flows/${flow.id}`, {
          state: { copilotPrefill: { text: suggestion.build_prompt, mode: 'build' } },
        });
      } catch (e) {
        log('onBuild: createFlow failed err=%o', e);
        setError(t('flows.suggest.error'));
      } finally {
        setOpeningId(null);
      }
    },
    [openingId, navigate, removeSuggestion, t]
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
              opening={openingId === suggestion.id}
              buildInProgress={openingId !== null}
              onBuild={() => void onBuild(suggestion)}
              onDismiss={() => void onDismiss(suggestion.id)}
            />
          ))}
        </div>
      )}
    </section>
  );
}
