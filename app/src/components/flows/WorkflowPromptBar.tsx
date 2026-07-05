/**
 * WorkflowPromptBar (Phase 5c) — the prompt-first authoring surface at the top
 * of the Flows page (and its empty-state hero). The user describes a workflow in
 * natural language; submitting spawns a `workflow_builder` turn in a DEDICATED
 * thread (via {@link useWorkflowBuilderChat}) and renders the returned proposal
 * inline with the existing {@link WorkflowProposalCard} ("Open in canvas" +
 * "Save & enable").
 *
 * Nothing here persists or enables a flow — the composer only asks the agent to
 * PROPOSE. Saving stays behind the card's explicit "Save & enable" click.
 */
import { useCallback, useState } from 'react';

import { useWorkflowBuilderChat } from '../../hooks/useWorkflowBuilderChat';
import { buildCreatePrompt } from '../../lib/flows/workflowBuilderPrompt';
import { useT } from '../../lib/i18n/I18nContext';
import WorkflowProposalCard from '../chat/WorkflowProposalCard';
import Button from '../ui/Button';

interface Props {
  /** Compact (list header) vs. hero (empty-state) presentation. */
  variant?: 'compact' | 'hero';
  /** Optional autofocus for the empty-state hero. */
  autoFocus?: boolean;
}

export default function WorkflowPromptBar({ variant = 'compact', autoFocus = false }: Props) {
  const { t } = useT();
  const { threadId, sending, proposal, error, send } = useWorkflowBuilderChat();
  const [text, setText] = useState('');

  const submit = useCallback(async () => {
    const trimmed = text.trim();
    if (!trimmed || sending) return;
    await send({ displayText: trimmed, prompt: buildCreatePrompt(trimmed) });
    setText('');
  }, [text, sending, send]);

  const onKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
      // Enter submits; Shift+Enter inserts a newline.
      if (event.key === 'Enter' && !event.shiftKey) {
        event.preventDefault();
        void submit();
      }
    },
    [submit]
  );

  const isHero = variant === 'hero';

  return (
    <section
      data-testid="workflow-prompt-bar"
      className={
        isHero
          ? 'rounded-2xl border border-ocean-200 bg-ocean-50/50 p-4 dark:border-ocean-700/50 dark:bg-ocean-500/5'
          : 'rounded-xl border border-line bg-surface p-3'
      }>
      <label htmlFor="workflow-prompt-input" className="sr-only">
        {t('flows.promptBar.label')}
      </label>
      {isHero && (
        <div className="mb-2">
          <h3 className="text-sm font-semibold text-content">{t('flows.promptBar.heroTitle')}</h3>
          <p className="text-xs text-content-muted">{t('flows.promptBar.heroSubtitle')}</p>
        </div>
      )}
      <div className="flex items-end gap-2">
        <textarea
          id="workflow-prompt-input"
          data-testid="workflow-prompt-input"
          value={text}
          onChange={e => setText(e.target.value)}
          onKeyDown={onKeyDown}
          rows={isHero ? 2 : 1}
          autoFocus={autoFocus}
          disabled={sending}
          placeholder={t('flows.promptBar.placeholder')}
          className="min-h-[38px] flex-1 resize-none rounded-lg border border-line bg-surface px-3 py-2 text-sm text-content placeholder:text-content-faint focus:border-ocean-400 focus:outline-none disabled:opacity-60"
        />
        <Button
          type="button"
          variant="primary"
          size="sm"
          data-testid="workflow-prompt-submit"
          disabled={sending || text.trim().length === 0}
          onClick={() => void submit()}>
          {sending ? t('flows.promptBar.thinking') : t('flows.promptBar.submit')}
        </Button>
      </div>

      {error && (
        <p className="mt-2 text-xs text-coral" data-testid="workflow-prompt-error">
          {error === 'offline' ? t('flows.promptBar.offline') : t('flows.promptBar.error')}
        </p>
      )}

      {threadId && proposal && (
        <div className="mt-3" data-testid="workflow-prompt-proposal">
          <WorkflowProposalCard threadId={threadId} proposal={proposal} />
        </div>
      )}
    </section>
  );
}
