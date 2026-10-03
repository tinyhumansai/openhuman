/**
 * "Add a source" — register a Documents source for memory to sync: a folder,
 * a file, a link, a GitHub repo, an RSS feed or a Composio toolkit, with an
 * optional label and sync schedule (`memory_sources_add`).
 */
import { useId, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import {
  DOCUMENT_SOURCE_KINDS,
  type DocumentSourceKind,
  type SourceAddRequest,
} from '../../services/api/memoryApi';
import { Button, Label, ModalShell, NativeSelect, TextField } from '../ui';
import { parsePositiveInt } from './memoryFormat';
import { sourceKindLabel, sourceTargetHint } from './memorySourceLabels';

interface MemoryAddSourceDialogProps {
  saving: boolean;
  error: string | null;
  /** Persist the source; resolves true when the core accepted it. */
  onSubmit: (req: SourceAddRequest) => Promise<boolean>;
  onClose: () => void;
}

export default function MemoryAddSourceDialog({
  saving,
  error,
  onSubmit,
  onClose,
}: MemoryAddSourceDialogProps) {
  const { t } = useT();
  const baseId = useId();
  const [kind, setKind] = useState<DocumentSourceKind>('folder');
  const [target, setTarget] = useState('');
  const [label, setLabel] = useState('');
  const [schedule, setSchedule] = useState('');

  const scheduleValue = schedule.trim() ? parsePositiveInt(schedule) : undefined;
  const canSubmit = !saving && target.trim().length > 0 && scheduleValue !== null;

  const submit = async () => {
    if (!canSubmit) return;
    const req: SourceAddRequest = { kind, target: target.trim() };
    if (label.trim()) req.label = label.trim();
    if (typeof scheduleValue === 'number') req.schedule_mins = scheduleValue;
    if (await onSubmit(req)) onClose();
  };

  return (
    <ModalShell
      title={t('memoryPage.documents.addTitle')}
      titleId={`${baseId}-title`}
      subtitle={t('memoryPage.documents.addSubtitle')}
      onClose={onClose}
      maxWidthClassName="max-w-md"
      testId="memory-add-source"
      footer={
        <div className="flex justify-end gap-2">
          <Button type="button" variant="secondary" size="sm" onClick={onClose} disabled={saving}>
            {t('common.cancel')}
          </Button>
          <Button
            type="button"
            variant="primary"
            size="sm"
            data-testid="memory-add-source-submit"
            disabled={!canSubmit}
            onClick={() => void submit()}>
            {t('memoryPage.documents.add')}
          </Button>
        </div>
      }>
      <form
        className="flex flex-col gap-4"
        onSubmit={event => {
          event.preventDefault();
          void submit();
        }}>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={`${baseId}-kind`} className="text-xs text-content-secondary">
            {t('memoryPage.documents.kindLabel')}
          </Label>
          <NativeSelect
            id={`${baseId}-kind`}
            data-testid="memory-add-source-kind"
            value={kind}
            disabled={saving}
            onChange={e => setKind(e.target.value as DocumentSourceKind)}>
            {DOCUMENT_SOURCE_KINDS.map(k => (
              <option key={k} value={k}>
                {sourceKindLabel(k, t)}
              </option>
            ))}
          </NativeSelect>
        </div>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={`${baseId}-target`} className="text-xs text-content-secondary">
            {t('memoryPage.documents.targetLabel')}
          </Label>
          <TextField
            id={`${baseId}-target`}
            data-testid="memory-add-source-target"
            mono
            spellCheck={false}
            autoFocus
            value={target}
            disabled={saving}
            placeholder={sourceTargetHint(kind, t)}
            onChange={e => setTarget(e.target.value)}
          />
        </div>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={`${baseId}-label`} className="text-xs text-content-secondary">
            {t('memoryPage.documents.labelLabel')}
          </Label>
          <TextField
            id={`${baseId}-label`}
            data-testid="memory-add-source-label"
            value={label}
            disabled={saving}
            onChange={e => setLabel(e.target.value)}
          />
        </div>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={`${baseId}-schedule`} className="text-xs text-content-secondary">
            {t('memoryPage.documents.scheduleLabel')}
          </Label>
          <TextField
            id={`${baseId}-schedule`}
            data-testid="memory-add-source-schedule"
            type="number"
            inputMode="numeric"
            min={1}
            value={schedule}
            disabled={saving}
            invalid={scheduleValue === null}
            placeholder={t('memoryPage.documents.schedulePlaceholder')}
            onChange={e => setSchedule(e.target.value)}
          />
          <p className="text-[11px] leading-4 text-content-muted">
            {t('memoryPage.documents.scheduleHelp')}
          </p>
        </div>
        {error !== null && (
          <p className="text-xs text-coral-600" role="alert" data-testid="memory-add-source-error">
            {error}
          </p>
        )}
      </form>
    </ModalShell>
  );
}
