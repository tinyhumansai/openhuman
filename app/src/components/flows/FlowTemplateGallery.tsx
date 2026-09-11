/**
 * FlowTemplateGallery (Phase 4c) — the curated-template picker. Presentational
 * only: it renders the bundled `FLOW_TEMPLATES` as selectable cards and calls
 * `onSelect` with the chosen template. The *create* side effect
 * (`flows_create` + navigate into the canvas) lives in the caller
 * (`NewWorkflowModal` / `FlowsPage`), so this component is trivially testable
 * and reusable both inside the new-workflow modal and inline on the Workflows
 * empty state.
 *
 * Display strings are i18n'd via the `templateNameKey`/`templateDescriptionKey`/
 * `templateCategoryKey` helpers — no template English is hardcoded here.
 */
import createDebug from 'debug';

import {
  FLOW_TEMPLATES,
  type FlowTemplate,
  templateCategoryKey,
  templateDescriptionKey,
  templateNameKey,
} from '../../lib/flows/templates';
import { useT } from '../../lib/i18n/I18nContext';
import { Badge, Button } from '../ui';

const log = createDebug('app:flows:templates');

interface FlowTemplateGalleryProps {
  /** Called with the picked template; the caller performs the create + navigate. */
  onSelect: (template: FlowTemplate) => void;
  /** Template id currently being created (shows a spinner label, disables the grid). */
  busyId?: string | null;
}

export default function FlowTemplateGallery({ onSelect, busyId }: FlowTemplateGalleryProps) {
  const { t } = useT();

  if (FLOW_TEMPLATES.length === 0) {
    return (
      <p className="py-6 text-center text-sm text-content-muted" data-testid="flow-templates-empty">
        {t('flows.templates.empty')}
      </p>
    );
  }

  return (
    <div
      data-testid="flow-template-gallery"
      className="grid grid-cols-1 gap-3 sm:grid-cols-2"
      role="list">
      {FLOW_TEMPLATES.map(template => {
        const busy = busyId === template.id;
        const anyBusy = Boolean(busyId);
        return (
          <Button
            key={template.id}
            type="button"
            variant="secondary"
            role="listitem"
            data-testid={`flow-template-${template.id}`}
            disabled={anyBusy}
            onClick={() => {
              log('template selected: id=%s', template.id);
              onSelect(template);
            }}
            className="h-auto flex-col items-start gap-1.5 rounded-2xl p-4 text-left font-normal hover:border-primary-300 hover:bg-primary-50/40 dark:hover:bg-primary-500/10">
            <Badge variant="primary" className="rounded-full">
              {t(templateCategoryKey(template.category))}
            </Badge>
            <span className="text-sm font-semibold text-content">
              {t(templateNameKey(template.id))}
            </span>
            <span className="text-xs leading-relaxed text-content-muted">
              {t(templateDescriptionKey(template.id))}
            </span>
            <span className="mt-1 text-xs font-medium text-primary-600 dark:text-primary-400">
              {busy ? t('flows.chooser.creating') : t('flows.templates.use')}
            </span>
          </Button>
        );
      })}
    </div>
  );
}
