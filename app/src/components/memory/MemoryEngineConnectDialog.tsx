/**
 * "Connect {engine}" — where a self-hosted engine's endpoint and API key are
 * typed (CortexDB at launch). Mirrors the web-search connect dialog: keys are
 * entered rarely, so they live in a dialog rather than inline in the list.
 */
import { useId, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { EngineDescriptor, EngineSetRequest } from '../../services/api/memoryApi';
import { Button, Label, ModalShell, TextField } from '../ui';
import { fill } from './memoryFormat';

interface MemoryEngineConnectDialogProps {
  engine: EngineDescriptor;
  /** The endpoint currently configured for this engine, when it is active. */
  currentEndpoint?: string;
  /** A key is already stored for this engine. */
  keySaved: boolean;
  saving: boolean;
  /** Persist the selection; resolves true when the core accepted it. */
  onSubmit: (req: EngineSetRequest) => Promise<boolean>;
  onClose: () => void;
}

export default function MemoryEngineConnectDialog({
  engine,
  currentEndpoint,
  keySaved,
  saving,
  onSubmit,
  onClose,
}: MemoryEngineConnectDialogProps) {
  const { t } = useT();
  const baseId = useId();
  const [endpoint, setEndpoint] = useState(currentEndpoint || engine.default_endpoint || '');
  const [apiKey, setApiKey] = useState('');

  // A self-hosted engine always takes an endpoint, even one with a default
  // (CortexDB defaults to its cloud): otherwise a local server is unreachable.
  const showEndpoint = engine.needs_endpoint || !engine.hosted;
  const keyRequired = engine.needs_key && !keySaved;
  const canSubmit =
    !saving &&
    (!showEndpoint || endpoint.trim().length > 0) &&
    (!keyRequired || apiKey.trim().length > 0);

  const submit = async () => {
    if (!canSubmit) return;
    const req: EngineSetRequest = { engine: engine.id };
    if (showEndpoint) req.endpoint = endpoint.trim();
    if (apiKey.trim()) req.api_key = apiKey.trim();
    if (await onSubmit(req)) onClose();
  };

  return (
    <ModalShell
      title={fill(t('memoryPage.engine.connectTitle'), { engine: engine.label })}
      titleId={`${baseId}-title`}
      subtitle={engine.description}
      onClose={onClose}
      maxWidthClassName="max-w-md"
      testId={`memory-engine-connect-${engine.id}`}
      footer={
        <div className="flex justify-end gap-2">
          <Button type="button" variant="secondary" size="sm" onClick={onClose} disabled={saving}>
            {t('common.cancel')}
          </Button>
          <Button
            type="button"
            variant="primary"
            size="sm"
            data-testid={`memory-engine-connect-${engine.id}-submit`}
            disabled={!canSubmit}
            onClick={() => void submit()}>
            {t('memoryPage.engine.connect')}
          </Button>
        </div>
      }>
      <form
        className="flex flex-col gap-4"
        onSubmit={event => {
          event.preventDefault();
          void submit();
        }}>
        {showEndpoint && (
          <div className="flex flex-col gap-1.5">
            <Label htmlFor={`${baseId}-endpoint`} className="text-xs text-content-secondary">
              {t('memoryPage.engine.endpoint')}
            </Label>
            <TextField
              id={`${baseId}-endpoint`}
              data-testid={`memory-engine-connect-${engine.id}-endpoint`}
              type="url"
              mono
              spellCheck={false}
              value={endpoint}
              disabled={saving}
              placeholder={engine.default_endpoint ?? 'https://'}
              onChange={e => setEndpoint(e.target.value)}
            />
          </div>
        )}
        {engine.needs_key && (
          <div className="flex flex-col gap-1.5">
            <Label htmlFor={`${baseId}-key`} className="text-xs text-content-secondary">
              {t('memoryPage.engine.apiKey')}
            </Label>
            <TextField
              id={`${baseId}-key`}
              data-testid={`memory-engine-connect-${engine.id}-key`}
              type="password"
              mono
              autoComplete="off"
              spellCheck={false}
              data-lpignore="true"
              data-1p-ignore="true"
              value={apiKey}
              disabled={saving}
              placeholder={keySaved ? t('memoryPage.engine.keySavedPlaceholder') : ''}
              onChange={e => setApiKey(e.target.value)}
            />
            {keySaved && (
              <p className="text-[11px] leading-4 text-content-muted">
                {t('memoryPage.engine.keySavedHint')}
              </p>
            )}
          </div>
        )}
      </form>
    </ModalShell>
  );
}
