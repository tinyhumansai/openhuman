/**
 * NewWorkflowModal (Phase 4a) — the "New workflow" chooser. Replaces the old
 * FlowsPage `/chat` TODO with three ways to start:
 *
 *  - **Start from scratch** — `flows_create` a blank graph carrying a single
 *    `manual` trigger, then open it in the editable canvas.
 *  - **From a template** — switch to the {@link FlowTemplateGallery} view;
 *    picking a card creates a flow from that template's graph and opens it.
 *
 * The old "Describe it" option was dropped: the Flows page already shows the
 * `WorkflowPromptBar` at all times (hero when empty, compact otherwise), so a
 * modal option that only re-focused that same bar was redundant.
 *
 * Create + navigate is delegated to {@link useCreateFlow}; this component only
 * owns which view is showing and assembles the name/graph for each path.
 */
import createDebug from 'debug';
import { useState } from 'react';

import { createBlankWorkflowGraph } from '../../lib/flows/newFlow';
import { type FlowTemplate, templateNameKey } from '../../lib/flows/templates';
import { useT } from '../../lib/i18n/I18nContext';
import { Alert, AlertDescription, Button, ModalShell } from '../ui';
import FlowTemplateGallery from './FlowTemplateGallery';
import { BLANK_FLOW_KEY, useCreateFlow } from './useCreateFlow';

interface NewWorkflowModalProps {
  onClose: () => void;
}

type View = 'chooser' | 'gallery';

/** One big tap-target row in the chooser view. */
function ChooserOption({
  testId,
  title,
  description,
  disabled,
  onClick,
}: {
  testId: string;
  title: string;
  description: string;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <Button
      type="button"
      variant="secondary"
      data-testid={testId}
      disabled={disabled}
      onClick={onClick}
      className="h-auto w-full flex-col items-start gap-0.5 rounded-2xl p-4 text-left font-normal hover:border-primary-300 hover:bg-primary-50/40 dark:hover:bg-primary-500/10">
      <span className="text-sm font-semibold text-content">{title}</span>
      <span className="text-xs leading-relaxed text-content-muted">{description}</span>
    </Button>
  );
}

const log = createDebug('app:flows:new');

export default function NewWorkflowModal({ onClose }: NewWorkflowModalProps) {
  const { t } = useT();
  const [view, setView] = useState<View>('chooser');
  const { create, busyKey, error, clearError } = useCreateFlow();

  const startFromScratch = () => {
    const name = t('flows.page.newWorkflow');
    const graph = createBlankWorkflowGraph(name, t('flows.nodeKind.trigger'));
    log('start from scratch');
    void create(BLANK_FLOW_KEY, name, graph);
  };

  const startFromTemplate = (template: FlowTemplate) => {
    const name = t(templateNameKey(template.id));
    log('start from template: id=%s', template.id);
    void create(template.id, name, template.graph);
  };

  const openGallery = () => {
    clearError();
    setView('gallery');
  };

  const backToChooser = () => {
    clearError();
    setView('chooser');
  };

  const busy = Boolean(busyKey);
  const title = view === 'gallery' ? t('flows.templates.title') : t('flows.chooser.title');
  const subtitle = view === 'gallery' ? t('flows.templates.subtitle') : t('flows.chooser.subtitle');

  return (
    <ModalShell
      onClose={onClose}
      title={title}
      subtitle={subtitle}
      titleId="new-workflow-modal-title"
      maxWidthClassName={view === 'gallery' ? 'max-w-2xl' : 'max-w-md'}>
      <div className="space-y-3" data-testid="new-workflow-modal">
        {error && (
          <Alert variant="destructive" density="compact" data-testid="new-workflow-error">
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}

        {view === 'chooser' ? (
          <>
            <ChooserOption
              testId="new-workflow-scratch"
              title={t('flows.chooser.scratchTitle')}
              description={t('flows.chooser.scratchDescription')}
              disabled={busy}
              onClick={startFromScratch}
            />
            <ChooserOption
              testId="new-workflow-template"
              title={t('flows.chooser.templateTitle')}
              description={t('flows.chooser.templateDescription')}
              disabled={busy}
              onClick={openGallery}
            />
          </>
        ) : (
          <>
            <Button
              type="button"
              variant="tertiary"
              size="xs"
              data-testid="new-workflow-gallery-back"
              onClick={backToChooser}
              className="h-auto px-0 text-xs font-medium text-primary-600 hover:bg-transparent hover:underline dark:text-primary-400">
              {t('flows.templates.back')}
            </Button>
            <FlowTemplateGallery onSelect={startFromTemplate} busyId={busyKey} />
          </>
        )}
      </div>
    </ModalShell>
  );
}
