/**
 * WorkflowPromptBar — the prompt-first authoring surface at the top of the
 * Flows page (and its empty-state hero). The user describes a workflow in
 * natural language; submitting IMMEDIATELY creates a blank flow (named from
 * the description) via `flows_create` and navigates into its canvas at
 * `/flows/:id` with a `copilotBuild` seed in `location.state`, so the canvas
 * opens with the copilot panel already running the build turn. The UI reacts
 * instantly instead of holding the user on the list page while the builder
 * agent works invisibly on a hidden thread.
 *
 * The copilot's proposal keeps the usual gates: the agent only PROPOSES; the
 * user Accepts the diff and the canvas's explicit Save persists the graph.
 */
import createDebug from 'debug';
import { useCallback, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { createBlankWorkflowGraph, deriveWorkflowName } from '../../lib/flows/newFlow';
import { useT } from '../../lib/i18n/I18nContext';
import { createFlow } from '../../services/api/flowsApi';
import Button from '../ui/Button';

const log = createDebug('app:flows:prompt-bar');

interface Props {
  /** Compact (list header) vs. hero (empty-state) presentation. */
  variant?: 'compact' | 'hero';
  /** Optional autofocus for the empty-state hero. */
  autoFocus?: boolean;
}

export default function WorkflowPromptBar({ variant = 'compact', autoFocus = false }: Props) {
  const { t } = useT();
  const navigate = useNavigate();
  const [text, setText] = useState('');
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = useCallback(async () => {
    const trimmed = text.trim();
    if (!trimmed || creating) return;
    setCreating(true);
    setError(null);
    const name = deriveWorkflowName(trimmed, t('flows.page.newWorkflow'));
    log('submit: creating flow name=%s', name);
    try {
      // Safe default: prompt-authored flows require approval so outbound
      // Slack/Gmail/HTTP/code nodes cannot fire unattended. Omitting this
      // arg would fall back to the server default of `false`.
      const flow = await createFlow(
        name,
        createBlankWorkflowGraph(name, t('flows.nodeKind.trigger')),
        true
      );
      log('submit: created id=%s — opening canvas with build seed', flow.id);
      navigate(`/flows/${flow.id}`, { state: { copilotBuild: { description: trimmed } } });
    } catch (err) {
      log('submit: create failed err=%o', err);
      setError(t('flows.promptBar.error'));
      setCreating(false);
    }
  }, [text, creating, navigate, t]);

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
          disabled={creating}
          placeholder={t('flows.promptBar.placeholder')}
          className="min-h-[38px] flex-1 resize-none rounded-lg border border-line bg-surface px-3 py-2 text-sm text-content placeholder:text-content-faint focus:border-ocean-400 focus:outline-none disabled:opacity-60"
        />
        <Button
          type="button"
          variant="primary"
          size="sm"
          data-testid="workflow-prompt-submit"
          disabled={creating || text.trim().length === 0}
          onClick={() => void submit()}>
          {creating ? t('flows.promptBar.thinking') : t('flows.promptBar.submit')}
        </Button>
      </div>

      {error && (
        <p className="mt-2 text-xs text-coral" data-testid="workflow-prompt-error">
          {error}
        </p>
      )}
    </section>
  );
}
