/**
 * WorkflowCopilotPanel (Phase 5c) — a side-panel chat bound to the
 * `workflow_builder` specialist, docked on the editable canvas. The user asks
 * for changes ("add a Slack notification on failure", "make the schedule
 * weekdays only"); each turn injects the CURRENT draft graph as context and the
 * agent returns a `revise_workflow` proposal. The panel surfaces the proposal's
 * node-level diff and hands Accept/Reject up to the host, which applies it to
 * the local draft overlay — a `revise_workflow` loop.
 *
 * Invariant: the copilot only PROPOSES. Accept applies to the UNSAVED local
 * draft (no `flows_update`); persistence stays behind the canvas's own Save.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import { useWorkflowBuilderChat } from '../../hooks/useWorkflowBuilderChat';
import { diffGraphs } from '../../lib/flows/graphDiff';
import type { WorkflowGraph } from '../../lib/flows/types';
import {
  buildRepairPrompt,
  buildRevisePrompt,
  type RepairPromptContext,
} from '../../lib/flows/workflowBuilderPrompt';
import { useT } from '../../lib/i18n/I18nContext';
import type { WorkflowProposal } from '../../store/chatRuntimeSlice';
import Button from '../ui/Button';

interface Props {
  /** The current draft graph, injected as context for each revise turn. */
  graph: WorkflowGraph;
  /**
   * Fires when the agent returns a fresh proposal, so the host can enter its
   * diff-preview overlay. The host computes/holds the preview; this panel only
   * reflects it.
   */
  onProposal: (proposal: WorkflowProposal) => void;
  /** Accept the pending proposal into the local draft (host commits it). */
  onAccept: (proposal: WorkflowProposal) => void;
  /** Reject the pending proposal (host reverts the overlay). */
  onReject: () => void;
  /** Close the panel. */
  onClose: () => void;
  /**
   * Optional repair seed (from a failed run's "Fix with agent") — auto-sends a
   * repair turn once on mount so the copilot opens already diagnosing.
   */
  repairSeed?: RepairPromptContext | null;
}

export default function WorkflowCopilotPanel({
  graph,
  onProposal,
  onAccept,
  onReject,
  onClose,
  repairSeed = null,
}: Props) {
  const { t } = useT();
  const { sending, proposal, error, send, clearProposal } = useWorkflowBuilderChat();
  const [text, setText] = useState('');

  // Surface each NEW proposal to the host exactly once (enter preview overlay).
  const lastSurfacedRef = useRef<WorkflowProposal | null>(null);
  useEffect(() => {
    if (proposal && proposal !== lastSurfacedRef.current) {
      lastSurfacedRef.current = proposal;
      onProposal(proposal);
    }
  }, [proposal, onProposal]);

  // Auto-send the repair turn once when opened from a failed run.
  const repairSentRef = useRef(false);
  useEffect(() => {
    if (!repairSeed || repairSentRef.current) return;
    repairSentRef.current = true;
    void send({
      displayText: t('flows.copilot.repairDisplay'),
      prompt: buildRepairPrompt(repairSeed),
    });
  }, [repairSeed, send, t]);

  const submit = useCallback(async () => {
    const trimmed = text.trim();
    if (!trimmed || sending) return;
    await send({ displayText: trimmed, prompt: buildRevisePrompt(trimmed, graph) });
    setText('');
  }, [text, sending, send, graph]);

  const onKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (event.key === 'Enter' && !event.shiftKey) {
        event.preventDefault();
        void submit();
      }
    },
    [submit]
  );

  const accept = useCallback(() => {
    if (!proposal) return;
    onAccept(proposal);
    clearProposal();
    lastSurfacedRef.current = null;
  }, [proposal, onAccept, clearProposal]);

  const reject = useCallback(() => {
    onReject();
    clearProposal();
    lastSurfacedRef.current = null;
  }, [onReject, clearProposal]);

  const diff = proposal ? diffGraphs(graph, proposal.graph as WorkflowGraph) : null;

  return (
    <aside
      data-testid="workflow-copilot-panel"
      className="flex h-full w-full max-w-sm flex-col border-l border-line bg-surface">
      <header className="flex items-start gap-2 border-b border-line px-3 py-2.5">
        <div className="min-w-0 flex-1">
          <p className="text-sm font-semibold text-content">{t('flows.copilot.title')}</p>
          <p className="text-[11px] text-content-muted">{t('flows.copilot.subtitle')}</p>
        </div>
        <button
          type="button"
          data-testid="workflow-copilot-close"
          aria-label={t('flows.copilot.close')}
          onClick={onClose}
          className="shrink-0 rounded-full p-1.5 text-content-faint hover:bg-surface-hover hover:text-content-secondary">
          ✕
        </button>
      </header>

      <div className="flex-1 space-y-3 overflow-y-auto px-3 py-3 text-sm">
        {!proposal && !sending && (
          <p className="text-xs text-content-muted" data-testid="workflow-copilot-empty">
            {t('flows.copilot.emptyState')}
          </p>
        )}

        {sending && (
          <p className="text-xs text-content-muted" data-testid="workflow-copilot-thinking">
            {t('flows.copilot.thinking')}
          </p>
        )}

        {error && (
          <p className="text-xs text-coral" data-testid="workflow-copilot-error">
            {error === 'offline' ? t('flows.copilot.offline') : t('flows.copilot.error')}
          </p>
        )}

        {proposal && diff && (
          <div
            data-testid="workflow-copilot-proposal"
            className="rounded-xl border border-ocean-300 bg-surface p-3 dark:border-ocean-700">
            <p className="text-xs font-semibold text-ocean-900 dark:text-ocean-100">
              {proposal.name || t('flows.copilot.proposalTitle')}
            </p>
            <p className="mt-1 text-[11px] text-content-muted">{t('flows.copilot.previewHint')}</p>

            <div className="mt-2 flex flex-wrap gap-1.5 text-[11px]">
              {diff.addedNodeIds.size > 0 && (
                <span
                  data-testid="workflow-copilot-added"
                  className="rounded-full bg-sage-100 px-2 py-0.5 font-medium text-sage-700 dark:bg-sage-500/15 dark:text-sage-300">
                  {t('flows.copilot.added').replace('{count}', String(diff.addedNodeIds.size))}
                </span>
              )}
              {diff.removedNodeIds.size > 0 && (
                <span
                  data-testid="workflow-copilot-removed"
                  className="rounded-full bg-coral-100 px-2 py-0.5 font-medium text-coral-700 dark:bg-coral-500/15 dark:text-coral-300">
                  {t('flows.copilot.removed').replace('{count}', String(diff.removedNodeIds.size))}
                </span>
              )}
              {!diff.hasChanges && (
                <span className="text-content-faint">{t('flows.copilot.noChanges')}</span>
              )}
            </div>

            <div className="mt-3 flex items-center gap-2">
              <Button
                type="button"
                variant="primary"
                size="sm"
                data-testid="workflow-copilot-accept"
                onClick={accept}>
                {t('flows.copilot.accept')}
              </Button>
              <Button
                type="button"
                variant="secondary"
                size="sm"
                data-testid="workflow-copilot-reject"
                onClick={reject}>
                {t('flows.copilot.reject')}
              </Button>
            </div>
          </div>
        )}
      </div>

      <div className="border-t border-line px-3 py-2.5">
        <label htmlFor="workflow-copilot-input" className="sr-only">
          {t('flows.copilot.placeholder')}
        </label>
        <div className="flex items-end gap-2">
          <textarea
            id="workflow-copilot-input"
            data-testid="workflow-copilot-input"
            value={text}
            onChange={e => setText(e.target.value)}
            onKeyDown={onKeyDown}
            rows={2}
            disabled={sending}
            placeholder={t('flows.copilot.placeholder')}
            className="min-h-[42px] flex-1 resize-none rounded-lg border border-line bg-surface px-3 py-2 text-sm text-content placeholder:text-content-faint focus:border-ocean-400 focus:outline-none disabled:opacity-60"
          />
          <Button
            type="button"
            variant="primary"
            size="sm"
            data-testid="workflow-copilot-send"
            disabled={sending || text.trim().length === 0}
            onClick={() => void submit()}>
            {sending ? t('flows.copilot.thinking') : t('flows.copilot.send')}
          </Button>
        </div>
      </div>
    </aside>
  );
}
