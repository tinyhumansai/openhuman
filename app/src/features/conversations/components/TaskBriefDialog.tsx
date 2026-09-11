import createDebug from 'debug';
import { useState } from 'react';
import { LuExternalLink, LuX } from 'react-icons/lu';

import Button from '../../../components/ui/Button';
import { DialogContent, DialogRoot, DialogTitle } from '../../../components/ui/Dialog';
import NativeSelect from '../../../components/ui/NativeSelect';
import TextArea from '../../../components/ui/TextArea';
import TextField from '../../../components/ui/TextField';
import { useT } from '../../../lib/i18n/I18nContext';
import type { TaskBoardCard, TaskBoardCardStatus } from '../../../types/turnState';
import { COLUMN_STATUSES, STATUS_LABEL_KEYS } from './taskBoardColumns';
import {
  emptyToNull,
  formatUrgency,
  joinLines,
  providerLabel,
  readSourceMetadata,
  splitLines,
  type TaskSourceMetadata,
} from './taskBoardMetadata';

const log = createDebug('app:conversations:task-brief');

const FIELD_LABEL = 'mb-1 block text-xs font-semibold text-content-muted';

/**
 * The per-card "Task brief" — read-only detail, or the full editor when the
 * caller supplied `onUpdate`.
 *
 * Extracted out of `TaskKanbanBoard.tsx` when that file was split, and moved
 * onto the shared Radix {@link DialogRoot}: the hand-rolled `createPortal` +
 * fixed scrim it replaced had no focus trap, no Escape handling, no scroll
 * lock and no focus restore — a modal editor is the single worst place in the
 * app to be missing those. The inputs are the shared
 * {@link TextField} / {@link TextArea} / {@link NativeSelect} primitives; each
 * still sits inside its own `<label>`, so every field keeps its accessible
 * name.
 */
export function TaskBriefDialog({
  card,
  disabled,
  onClose,
  onUpdate,
  onDelete,
}: {
  card: TaskBoardCard;
  disabled: boolean;
  onClose: () => void;
  onUpdate?: (card: TaskBoardCard, nextCard: TaskBoardCard) => void;
  onDelete?: (card: TaskBoardCard) => void;
}) {
  const { t } = useT();
  const source = readSourceMetadata(card.sourceMetadata);
  const editable = Boolean(onUpdate) && !disabled;
  const deletable = Boolean(onDelete) && !disabled;

  const [title, setTitle] = useState(card.title);
  const [status, setStatus] = useState<TaskBoardCardStatus>(card.status);
  const [objective, setObjective] = useState(card.objective ?? '');
  const [assignedAgent, setAssignedAgent] = useState(card.assignedAgent ?? '');
  const [approvalMode, setApprovalMode] = useState(card.approvalMode ?? '');
  const [plan, setPlan] = useState(joinLines(card.plan));
  const [allowedTools, setAllowedTools] = useState(joinLines(card.allowedTools));
  const [acceptanceCriteria, setAcceptanceCriteria] = useState(joinLines(card.acceptanceCriteria));
  const [evidence, setEvidence] = useState(joinLines(card.evidence));
  const [notes, setNotes] = useState(card.notes ?? '');
  const [blocker, setBlocker] = useState(card.blocker ?? '');

  const handleDelete = () => {
    if (!deletable) return;
    log('delete card=%s', card.id);
    onDelete?.(card);
    onClose();
  };

  const save = () => {
    if (!editable) return;
    const trimmedTitle = title.trim();
    if (!trimmedTitle) return;
    log('save card=%s status=%s', card.id, status);
    onUpdate?.(card, {
      ...card,
      title: trimmedTitle,
      status,
      objective: emptyToNull(objective),
      assignedAgent: emptyToNull(assignedAgent),
      approvalMode:
        approvalMode === 'required' || approvalMode === 'not_required' ? approvalMode : null,
      plan: splitLines(plan),
      allowedTools: splitLines(allowedTools),
      acceptanceCriteria: splitLines(acceptanceCriteria),
      evidence: splitLines(evidence),
      notes: emptyToNull(notes),
      blocker: emptyToNull(blocker),
    });
    onClose();
  };

  const statusOptions = COLUMN_STATUSES.includes(status)
    ? COLUMN_STATUSES
    : [status, ...COLUMN_STATUSES];

  return (
    // `open` is hard-coded: the board only mounts this component while a card
    // is selected. `onOpenChange` routes Escape / outside-click to `onClose`.
    <DialogRoot
      open
      onOpenChange={next => {
        if (!next) onClose();
      }}>
      <DialogContent
        aria-describedby={undefined}
        data-testid="task-brief-dialog"
        className="max-h-[calc(100vh-3rem)] max-w-xl overflow-y-auto border border-line p-4">
        <div className="mb-3 flex items-start justify-between gap-3">
          <div className="min-w-0">
            <p className="text-[11px] font-semibold uppercase text-content-faint">
              {t('conversations.taskKanban.briefTitle')}
            </p>
            {/* `asChild` keeps the historical h3 so `getByRole('heading')` still
                resolves the card title. */}
            <DialogTitle asChild>
              <h3 className="wrap-break-word text-base font-semibold text-content">{card.title}</h3>
            </DialogTitle>
          </div>
          <Button
            iconOnly
            variant="tertiary"
            size="sm"
            aria-label={t('conversations.taskKanban.closeBrief')}
            onClick={onClose}
            className="flex-none">
            <LuX className="h-4 w-4" />
          </Button>
        </div>

        {source && <SourceBrief source={source} />}

        {editable ? (
          <div className="space-y-3 text-sm">
            <label className="block">
              <span className={FIELD_LABEL}>{t('conversations.taskKanban.field.title')}</span>
              <TextField value={title} onChange={e => setTitle(e.target.value)} />
            </label>
            <div className="grid gap-3 sm:grid-cols-3">
              <label className="block">
                <span className={FIELD_LABEL}>{t('conversations.taskKanban.field.status')}</span>
                <NativeSelect
                  value={status}
                  onChange={e => setStatus(e.target.value as TaskBoardCardStatus)}
                  className="w-full">
                  {statusOptions.map(s => (
                    <option key={s} value={s}>
                      {t(STATUS_LABEL_KEYS[s])}
                    </option>
                  ))}
                </NativeSelect>
              </label>
              <BriefInput
                label={t('conversations.taskKanban.field.assignedAgent')}
                value={assignedAgent}
                onChange={setAssignedAgent}
              />
              <label className="block">
                <span className={FIELD_LABEL}>{t('conversations.taskKanban.field.approval')}</span>
                <NativeSelect
                  value={approvalMode}
                  onChange={e => setApprovalMode(e.target.value)}
                  className="w-full">
                  <option value="">{t('conversations.taskKanban.approval.default')}</option>
                  <option value="required">
                    {t('conversations.taskKanban.approval.required')}
                  </option>
                  <option value="not_required">
                    {t('conversations.taskKanban.approval.notRequired')}
                  </option>
                </NativeSelect>
              </label>
            </div>
            <BriefInput
              label={t('conversations.taskKanban.field.objective')}
              value={objective}
              onChange={setObjective}
            />
            <BriefTextarea
              label={t('conversations.taskKanban.field.plan')}
              value={plan}
              onChange={setPlan}
            />
            <BriefTextarea
              label={t('conversations.taskKanban.field.allowedTools')}
              value={allowedTools}
              onChange={setAllowedTools}
            />
            <BriefTextarea
              label={t('conversations.taskKanban.field.acceptanceCriteria')}
              value={acceptanceCriteria}
              onChange={setAcceptanceCriteria}
            />
            <BriefTextarea
              label={t('conversations.taskKanban.field.evidence')}
              value={evidence}
              onChange={setEvidence}
            />
            <BriefTextarea
              label={t('conversations.taskKanban.field.notes')}
              value={notes}
              onChange={setNotes}
            />
            <BriefTextarea
              label={t('conversations.taskKanban.field.blocker')}
              value={blocker}
              onChange={setBlocker}
            />
            <div className="flex items-center justify-between gap-2 pt-1">
              {deletable ? (
                <Button variant="secondary" tone="danger" size="sm" onClick={handleDelete}>
                  {t('conversations.taskKanban.deleteCard')}
                </Button>
              ) : (
                <span />
              )}
              <div className="flex gap-2">
                <Button variant="secondary" size="sm" onClick={onClose}>
                  {t('common.cancel')}
                </Button>
                <Button variant="primary" size="sm" onClick={save} disabled={!title.trim()}>
                  {t('conversations.taskKanban.saveChanges')}
                </Button>
              </div>
            </div>
          </div>
        ) : (
          <div className="space-y-4 text-sm">
            <BriefText
              label={t('conversations.taskKanban.field.objective')}
              value={card.objective}
            />
            <BriefText
              label={t('conversations.taskKanban.field.assignedAgent')}
              value={card.assignedAgent}
              mono
            />
            <BriefText
              label={t('conversations.taskKanban.field.approval')}
              value={
                card.approvalMode === 'required'
                  ? t('conversations.taskKanban.approval.requiredBeforeExecution')
                  : card.approvalMode === 'not_required'
                    ? t('conversations.taskKanban.approval.notRequired')
                    : undefined
              }
            />
            <BriefList
              label={t('conversations.taskKanban.field.plan')}
              values={card.plan}
              ordered
            />
            <BriefList
              label={t('conversations.taskKanban.field.allowedTools')}
              values={card.allowedTools}
              mono
            />
            <BriefList
              label={t('conversations.taskKanban.field.acceptanceCriteria')}
              values={card.acceptanceCriteria}
            />
            <BriefList
              label={t('conversations.taskKanban.field.evidence')}
              values={card.evidence}
            />
            <BriefText label={t('conversations.taskKanban.field.notes')} value={card.notes} />
            <BriefText
              label={t('conversations.taskKanban.field.blocker')}
              value={card.blocker}
              tone="danger"
            />
            {deletable && (
              <div className="flex justify-end pt-1">
                <Button variant="secondary" tone="danger" size="sm" onClick={handleDelete}>
                  {t('conversations.taskKanban.deleteCard')}
                </Button>
              </div>
            )}
          </div>
        )}
      </DialogContent>
    </DialogRoot>
  );
}

/**
 * The upstream task-source provenance panel. `sky-*` was a raw Tailwind
 * palette scale that does not follow a user's theme; this reads on the
 * semantic `primary-*` tokens now.
 */
function SourceBrief({ source }: { source: TaskSourceMetadata }) {
  const { t } = useT();
  const urgency = formatUrgency(source.urgency, t);

  return (
    <div className="mb-4 rounded-lg border border-primary-200 bg-primary-50 p-3 text-sm dark:border-primary-500/20 dark:bg-primary-500/10">
      <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
        <h4 className="text-xs font-semibold text-primary-800 dark:text-primary-100">
          {t('conversations.taskKanban.source.title')}
        </h4>
        {source.url && (
          <a
            href={source.url}
            target="_blank"
            rel="noreferrer"
            className="inline-flex items-center gap-1 text-xs font-medium text-primary-600 hover:text-primary-700 dark:text-primary-300 dark:hover:text-primary-200">
            <LuExternalLink className="h-3 w-3" />
            {t('conversations.taskKanban.source.openExternal')}
          </a>
        )}
      </div>
      <dl className="grid gap-2 sm:grid-cols-2">
        <SourceBriefField
          label={t('settings.taskSources.provider')}
          value={providerLabel(source.provider, t)}
        />
        <SourceBriefField
          label={t('conversations.taskKanban.source.sourceId')}
          value={source.sourceId}
          mono
        />
        <SourceBriefField
          label={t('conversations.taskKanban.source.externalId')}
          value={source.externalId}
          mono
        />
        <SourceBriefField label={t('conversations.taskKanban.source.repo')} value={source.repo} />
        <SourceBriefField label={t('conversations.taskKanban.source.urgency')} value={urgency} />
      </dl>
    </div>
  );
}

function SourceBriefField({
  label,
  value,
  mono = false,
}: {
  label: string;
  value?: string;
  mono?: boolean;
}) {
  if (!value) return null;
  return (
    <div className="min-w-0">
      <dt className="text-[11px] font-semibold text-primary-700 dark:text-primary-200">{label}</dt>
      <dd className={`mt-0.5 wrap-break-word text-xs text-content ${mono ? 'font-mono' : ''}`}>
        {value}
      </dd>
    </div>
  );
}

function BriefInput({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <label className="block">
      <span className={FIELD_LABEL}>{label}</span>
      <TextField value={value} onChange={e => onChange(e.target.value)} />
    </label>
  );
}

function BriefTextarea({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <label className="block">
      <span className={FIELD_LABEL}>{label}</span>
      <TextArea
        value={value}
        onChange={e => onChange(e.target.value)}
        rows={3}
        className="resize-y"
      />
    </label>
  );
}

function BriefText({
  label,
  value,
  mono = false,
  tone = 'default',
}: {
  label: string;
  value?: string | null;
  mono?: boolean;
  tone?: 'default' | 'danger';
}) {
  if (!value) return null;
  return (
    <div>
      <h4 className="mb-1 text-xs font-semibold text-content-muted">{label}</h4>
      <p
        className={`wrap-break-word text-sm ${
          mono ? 'font-mono' : ''
        } ${tone === 'danger' ? 'text-coral-600' : 'text-content'}`}>
        {value}
      </p>
    </div>
  );
}

function BriefList({
  label,
  values,
  ordered = false,
  mono = false,
}: {
  label: string;
  values?: string[];
  ordered?: boolean;
  mono?: boolean;
}) {
  if (!values?.length) return null;
  const List = ordered ? 'ol' : 'ul';
  return (
    <div>
      <h4 className="mb-1 text-xs font-semibold text-content-muted">{label}</h4>
      <List
        className={`space-y-1 ${
          ordered ? 'list-decimal' : 'list-disc'
        } list-inside text-sm text-content ${mono ? 'font-mono' : ''}`}>
        {values.map((value, index) => (
          <li key={index} className="wrap-break-word">
            {value}
          </li>
        ))}
      </List>
    </div>
  );
}
