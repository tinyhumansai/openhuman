import { ExternalLink, Info } from 'lucide-react';
import { type ReactNode, useId } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { type EngineState, isMemoryOn } from '../../services/api/memoryApi';
import { TINYHUMANS_TERMS_URL } from '../../utils/links';
import { openUrl } from '../../utils/openUrl';
import { Alert, AlertDescription, Button, Label, TextField } from '../ui';
import { PopoverContent, PopoverRoot, PopoverTrigger } from '../ui/Popover';
import type { MemoryProviderOption } from './MemoryProviderLogo';

/** CortexDB's self-hosting guide, linked from the Local provider. */
export const CORTEXDB_SELF_HOST_DOCS_URL = 'https://cortexdb.ai/docs/self-hosting/quickstart';

/** CortexDB's default port on this computer, shown as the example endpoint. */
const SELF_HOST_EXAMPLE_ENDPOINT = 'http://localhost:3141';

const ExternalTextLink = ({
  href,
  testId,
  children,
}: {
  href: string;
  testId: string;
  children: ReactNode;
}) => (
  <a
    href={href}
    target="_blank"
    rel="noopener noreferrer"
    data-testid={testId}
    onClick={event => {
      event.preventDefault();
      void openUrl(href).catch(() => undefined);
    }}
    className="inline-flex items-center gap-1 font-medium text-primary-600 hover:underline dark:text-primary-300">
    {children}
    <ExternalLink className="h-3 w-3" aria-hidden />
  </a>
);

export interface MemoryConnectionPanelProps {
  option: MemoryProviderOption;
  state: EngineState;
  /** True when this provider is the configured one. */
  active: boolean;
  signedIn: boolean;
  /** The signed-in user's plan, when known (FREE / BASIC / PRO). */
  plan: string | null;
  /** The option currently being saved, if any. */
  saving: MemoryProviderOption | null;
  error?: string;
  cloudKey: string;
  onCloudKey: (value: string) => void;
  localEndpoint: string;
  onLocalEndpoint: (value: string) => void;
  endpointInvalid: boolean;
  localKey: string;
  onLocalKey: (value: string) => void;
  /** Whether the footer's primary button can be pressed. */
  canSubmit: boolean;
  onSubmit: () => void;
}

/** Free hosted memory per plan; memory inference itself is never charged. */
const MEMORY_QUOTA = { BASIC: '1 GB', PRO: '20 GB' } as const;

/**
 * One way to connect CortexDB, shown under its chip on Memory → Provider: what
 * it is, what it needs (nothing via TinyHumans, a key for your own account, an
 * endpoint and key for Local), its action, and via TinyHumans one line on
 * who hosts it and that memory inference is free on the plan; the info popover holds the
 * details: the plan's quota (Basic 1 GB, Pro 20 GB), that memory inference is
 * never charged, and the fair-use terms.
 */
export default function MemoryConnectionPanel({
  option,
  state,
  active,
  signedIn,
  plan,
  saving,
  error,
  cloudKey,
  onCloudKey,
  localEndpoint,
  onLocalEndpoint,
  endpointInvalid,
  localKey,
  onLocalKey,
  canSubmit,
  onSubmit,
}: MemoryConnectionPanelProps) {
  const { t } = useT();
  const baseId = useId();
  const formId = `${baseId}-form`;
  const busy = saving !== null;
  const planName = plan === 'PRO' ? 'Pro' : plan === 'BASIC' ? 'Basic' : null;

  const keyField = (value: string, onChange: (value: string) => void) => {
    const saved = active && state.has_key;
    return (
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={`${baseId}-key`} className="text-xs text-content-secondary">
          {t('memoryPage.engine.apiKey')}
        </Label>
        <TextField
          id={`${baseId}-key`}
          data-testid={`memory-engine-${option}-key`}
          type="password"
          mono
          autoComplete="off"
          spellCheck={false}
          data-lpignore="true"
          data-1p-ignore="true"
          value={value}
          disabled={busy}
          placeholder={saved ? t('memoryPage.engine.keySavedPlaceholder') : ''}
          onChange={e => onChange(e.target.value)}
        />
        {saved && (
          <p className="text-[11px] leading-4 text-content-muted">
            {t('memoryPage.engine.keySavedHint')}
          </p>
        )}
      </div>
    );
  };

  const builtinBody = (
    <>
      <p className="text-xs leading-relaxed text-content-secondary">
        <span data-testid="memory-engine-builtin-note">
          {planName
            ? t('memoryPage.engine.builtin.summaryPlan').replace('{plan}', planName)
            : t('memoryPage.engine.builtin.summaryUpgrade')}
        </span>
        <PopoverRoot>
          <PopoverTrigger asChild>
            <Button
              size="xs"
              variant="tertiary"
              iconOnly
              analyticsId="memory-engine-fair-use"
              data-testid="memory-engine-fair-use-trigger"
              aria-label={t('memoryPage.engine.fairUse.summary')}
              title={t('memoryPage.engine.fairUse.summary')}
              className="ml-0.5 h-5 w-5 align-middle text-content-muted hover:text-content">
              <Info className="h-3.5 w-3.5" aria-hidden />
            </Button>
          </PopoverTrigger>
          <PopoverContent
            align="start"
            className="w-80 text-xs"
            data-testid="memory-engine-fair-use">
            <p className="font-semibold text-content">{t('memoryPage.engine.fairUse.summary')}</p>
            <ul
              className="mt-1.5 list-disc space-y-1 pl-4 leading-relaxed text-content-muted"
              data-testid="memory-engine-quota">
              <li>
                {planName
                  ? t('memoryPage.engine.quota.plan')
                      .replace('{plan}', planName)
                      .replace('{storage}', MEMORY_QUOTA[plan as 'BASIC' | 'PRO'])
                  : t('memoryPage.engine.quota.all')}
              </li>
              <li>{t('memoryPage.engine.quota.inference')}</li>
            </ul>
            <p className="mt-2.5 font-semibold text-content">
              {t('memoryPage.engine.fairUse.heading')}
            </p>
            <ul className="mt-1.5 list-disc space-y-1 pl-4 leading-relaxed text-content-muted">
              <li>{t('memoryPage.engine.fairUse.own')}</li>
              <li>{t('memoryPage.engine.fairUse.noAbuse')}</li>
              <li>{t('memoryPage.engine.fairUse.limits')}</li>
            </ul>
            <p className="mt-2">
              <ExternalTextLink href={TINYHUMANS_TERMS_URL} testId="memory-engine-terms">
                {t('memoryPage.engine.fairUse.terms')}
              </ExternalTextLink>
            </p>
          </PopoverContent>
        </PopoverRoot>
      </p>
      {!signedIn && (
        <p className="text-xs text-content-muted" data-testid="memory-engine-builtin-sign-in">
          {t('memoryPage.engine.builtin.signInHint')}
        </p>
      )}
    </>
  );

  const cloudBody = (
    <form
      id={formId}
      className="flex flex-col gap-3"
      onSubmit={event => {
        event.preventDefault();
        onSubmit();
      }}>
      <p className="text-xs leading-relaxed text-content-secondary">
        {t('memoryPage.engine.apiKeyOption.description')}
      </p>
      {keyField(cloudKey, onCloudKey)}
    </form>
  );

  const localBody = (
    <form
      id={formId}
      className="flex flex-col gap-3"
      onSubmit={event => {
        event.preventDefault();
        onSubmit();
      }}>
      <ol className="list-decimal space-y-1 pl-4 text-xs leading-relaxed text-content-secondary">
        <li>
          {t('memoryPage.engine.selfHost.step1')}{' '}
          <ExternalTextLink href={CORTEXDB_SELF_HOST_DOCS_URL} testId="memory-engine-selfhost-docs">
            {t('memoryPage.engine.selfHost.docsLink')}
          </ExternalTextLink>
        </li>
        <li>{t('memoryPage.engine.selfHost.step2')}</li>
        <li>{t('memoryPage.engine.selfHost.step3')}</li>
      </ol>
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={`${baseId}-endpoint`} className="text-xs text-content-secondary">
          {t('memoryPage.engine.endpoint')}
        </Label>
        <TextField
          id={`${baseId}-endpoint`}
          data-testid="memory-engine-selfhost-endpoint"
          type="url"
          mono
          spellCheck={false}
          value={localEndpoint}
          disabled={busy}
          placeholder={SELF_HOST_EXAMPLE_ENDPOINT}
          onChange={e => onLocalEndpoint(e.target.value)}
        />
        {endpointInvalid && (
          <p
            className="text-[11px] leading-4 text-destructive"
            data-testid="memory-engine-selfhost-endpoint-error">
            {t('memoryPage.engine.selfHost.notLocal')}
          </p>
        )}
      </div>
      {keyField(localKey, onLocalKey)}
    </form>
  );

  // TinyHumans has nothing to save once it is in use; the others always can
  // (a new key, a moved endpoint).
  const showSubmit = option !== 'builtin' || !(active && isMemoryOn(state));
  const submitLabel =
    saving === option
      ? t('memoryPage.engine.connecting')
      : option === 'builtin'
        ? t('memoryPage.engine.use')
        : active
          ? t('memoryPage.engine.save')
          : t('memoryPage.engine.connect');

  return (
    <div
      className="flex flex-col gap-2"
      role="tabpanel"
      data-testid={`memory-engine-panel-${option}`}>
      {option === 'builtin' ? builtinBody : option === 'apikey' ? cloudBody : localBody}
      {error && (
        <Alert variant="destructive" data-testid={`memory-engine-${option}-error`}>
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}
      {showSubmit && (
        <div className="flex items-center justify-end gap-2">
          <Button
            size="sm"
            type={option === 'builtin' ? 'button' : 'submit'}
            form={option === 'builtin' ? undefined : formId}
            analyticsId={`memory-engine-${option}-submit`}
            data-testid={`memory-engine-${option}-submit`}
            disabled={!canSubmit}
            onClick={option === 'builtin' ? onSubmit : undefined}>
            {submitLabel}
          </Button>
        </div>
      )}
    </div>
  );
}
