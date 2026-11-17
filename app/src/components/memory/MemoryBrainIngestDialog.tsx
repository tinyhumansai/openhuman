/**
 * "Add a document" — put one document into the shared brain
 * (`memory_brain_ingest`): pasted text or a file path on this device, with an
 * optional title and source type (left empty, the core files it by what it
 * finds: pdf, markdown, …).
 */
import { useId, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { type BrainIngestRequest, isOutOfCreditsMessage } from '../../services/api/memoryApi';
import {
  Button,
  Label,
  ModalShell,
  TextArea,
  TextField,
  ToggleGroupItem,
  ToggleGroupRoot,
} from '../ui';
import { MemoryCreditsPrompt } from './MemoryErrorAlert';

type IngestFrom = 'text' | 'path';

interface MemoryBrainIngestDialogProps {
  saving: boolean;
  error: string | null;
  /** Ingest the document; resolves true when the core accepted it. */
  onSubmit: (req: BrainIngestRequest) => Promise<boolean>;
  onClose: () => void;
}

export default function MemoryBrainIngestDialog({
  saving,
  error,
  onSubmit,
  onClose,
}: MemoryBrainIngestDialogProps) {
  const { t } = useT();
  const baseId = useId();
  const [from, setFrom] = useState<IngestFrom>('text');
  const [text, setText] = useState('');
  const [path, setPath] = useState('');
  const [title, setTitle] = useState('');
  const [source, setSource] = useState('');

  const body = from === 'text' ? text.trim() : path.trim();
  const canSubmit = !saving && body.length > 0;

  const submit = async () => {
    if (!canSubmit) return;
    const req: BrainIngestRequest = from === 'text' ? { text: body } : { path: body };
    if (title.trim()) req.title = title.trim();
    if (source.trim()) req.source = source.trim();
    if (await onSubmit(req)) onClose();
  };

  return (
    <ModalShell
      title={t('memoryPage.brain.ingestTitle')}
      titleId={`${baseId}-title`}
      subtitle={t('memoryPage.brain.ingestSubtitle')}
      onClose={onClose}
      maxWidthClassName="max-w-md"
      testId="memory-brain-ingest"
      footer={
        <div className="flex justify-end gap-2">
          <Button type="button" variant="secondary" size="sm" onClick={onClose} disabled={saving}>
            {t('common.cancel')}
          </Button>
          <Button
            type="button"
            variant="primary"
            size="sm"
            analyticsId="memory-brain-ingest-submit"
            data-testid="memory-brain-ingest-submit"
            disabled={!canSubmit}
            onClick={() => void submit()}>
            {t('memoryPage.brain.ingest')}
          </Button>
        </div>
      }>
      <form
        className="flex flex-col gap-4"
        onSubmit={event => {
          event.preventDefault();
          void submit();
        }}>
        <ToggleGroupRoot
          type="single"
          size="sm"
          value={from}
          onValueChange={next => {
            if (next) setFrom(next as IngestFrom);
          }}
          aria-label={t('memoryPage.brain.ingestFrom')}>
          <ToggleGroupItem value="text" data-testid="memory-brain-ingest-from-text">
            {t('memoryPage.brain.ingestFromText')}
          </ToggleGroupItem>
          <ToggleGroupItem value="path" data-testid="memory-brain-ingest-from-path">
            {t('memoryPage.brain.ingestFromPath')}
          </ToggleGroupItem>
        </ToggleGroupRoot>
        {from === 'text' ? (
          <div className="flex flex-col gap-1.5">
            <Label htmlFor={`${baseId}-text`} className="text-xs text-content-secondary">
              {t('memoryPage.brain.textLabel')}
            </Label>
            <TextArea
              id={`${baseId}-text`}
              data-testid="memory-brain-ingest-text"
              rows={6}
              value={text}
              disabled={saving}
              onChange={e => setText(e.target.value)}
            />
          </div>
        ) : (
          <div className="flex flex-col gap-1.5">
            <Label htmlFor={`${baseId}-path`} className="text-xs text-content-secondary">
              {t('memoryPage.brain.pathLabel')}
            </Label>
            <TextField
              id={`${baseId}-path`}
              data-testid="memory-brain-ingest-path"
              mono
              spellCheck={false}
              value={path}
              disabled={saving}
              placeholder={t('memoryPage.sourceTarget.file')}
              onChange={e => setPath(e.target.value)}
            />
          </div>
        )}
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={`${baseId}-title`} className="text-xs text-content-secondary">
            {t('memoryPage.brain.titleLabel')}
          </Label>
          <TextField
            id={`${baseId}-title`}
            data-testid="memory-brain-ingest-title"
            value={title}
            disabled={saving}
            onChange={e => setTitle(e.target.value)}
          />
        </div>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={`${baseId}-source`} className="text-xs text-content-secondary">
            {t('memoryPage.brain.sourceLabel')}
          </Label>
          <TextField
            id={`${baseId}-source`}
            data-testid="memory-brain-ingest-source"
            mono
            spellCheck={false}
            value={source}
            disabled={saving}
            placeholder={t('memoryPage.brain.sourcePlaceholder')}
            onChange={e => setSource(e.target.value)}
          />
        </div>
        {error !== null &&
          (isOutOfCreditsMessage(error, t) ? (
            <MemoryCreditsPrompt message={error} data-testid="memory-brain-ingest-error" />
          ) : (
            <p
              className="text-xs text-coral-600"
              role="alert"
              data-testid="memory-brain-ingest-error">
              {error}
            </p>
          ))}
      </form>
    </ModalShell>
  );
}
