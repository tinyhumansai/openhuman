/**
 * FlowValidationBanner (Phase 3c) — the inline error/warning surface for the
 * editable Workflow Canvas. Renders two distinct lists:
 *
 *  - **errors** (coral) — hard structural problems from `flows_validate`
 *    (`valid === false`): missing/duplicate trigger, cycle, invalid node config,
 *    unknown-node edge. These block Save (gated by the canvas, not here).
 *  - **warnings** (amber) — advisory notes that never block Save (today: an
 *    unfired-trigger-kind warning; see `flows/ops.rs::graph_trigger_warnings`).
 *
 * A `saveError` (the `flows_update` RPC itself failing) renders as a third,
 * separate error row so a transport failure reads differently from a graph
 * that's structurally invalid. When there's nothing to show the component
 * renders nothing.
 *
 * Presentational only — the canvas owns validation state and Save gating.
 */
import { memo } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import type { FlowValidation } from '../../../services/api/flowsApi';

export interface FlowValidationBannerProps {
  validation: FlowValidation | null;
  /** Message from a failed `flows_update` Save, shown as a distinct error row. */
  saveError?: string | null;
}

function MessageList({
  title,
  messages,
  tone,
  testId,
}: {
  title: string;
  messages: string[];
  tone: 'error' | 'warning';
  testId: string;
}) {
  const toneClasses =
    tone === 'error'
      ? 'border-coral-300/60 bg-coral-50 text-coral-700 dark:border-coral-500/40 dark:bg-coral-500/10 dark:text-coral-300'
      : 'border-amber-300/60 bg-amber-50 text-amber-700 dark:border-amber-500/40 dark:bg-amber-500/10 dark:text-amber-300';
  return (
    <div className={`rounded-lg border px-3 py-2 text-xs ${toneClasses}`} data-testid={testId}>
      <div className="mb-1 font-semibold uppercase tracking-wide">{title}</div>
      <ul className="space-y-0.5">
        {messages.map((message, i) => (
          <li key={`${message}-${i}`} className="leading-snug">
            {message}
          </li>
        ))}
      </ul>
    </div>
  );
}

function FlowValidationBanner({ validation, saveError }: FlowValidationBannerProps) {
  const { t } = useT();

  const errors = validation && !validation.valid ? validation.errors : [];
  const warnings = validation?.warnings ?? [];

  if (errors.length === 0 && warnings.length === 0 && !saveError) {
    return null;
  }

  return (
    <div
      className="flex max-h-40 w-full flex-col gap-2 overflow-y-auto"
      data-testid="flow-editor-validation">
      {saveError && (
        <MessageList
          title={t('flows.editor.saveFailedTitle')}
          messages={[saveError]}
          tone="error"
          testId="flow-editor-save-error"
        />
      )}
      {errors.length > 0 && (
        <MessageList
          title={t('flows.editor.errorsTitle')}
          messages={errors}
          tone="error"
          testId="flow-editor-errors"
        />
      )}
      {warnings.length > 0 && (
        <MessageList
          title={t('flows.editor.warningsTitle')}
          messages={warnings}
          tone="warning"
          testId="flow-editor-warnings"
        />
      )}
    </div>
  );
}

export default memo(FlowValidationBanner);
