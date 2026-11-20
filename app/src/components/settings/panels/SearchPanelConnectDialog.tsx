/*
 * "Connect {provider}" — the one place a search provider's API key or
 * instance URL is typed.
 *
 * The old panel rendered a key editor inline under every provider, on or off,
 * so the page opened on a column of empty password fields. Keys are entered
 * rarely and once; a dialog opened from the provider's tile or its row menu
 * keeps the list scannable and says, in its own title, which provider the key
 * belongs to.
 */
import { ExternalLink } from 'lucide-react';
import { useId, useState } from 'react';

import { openUrl } from '../../../utils/openUrl';
import type { SearchProviderInfo, SearchProviderUpdate } from '../../../utils/tauriCommands/config';
import Button from '../../ui/Button';
import Label from '../../ui/Label';
import { ModalShell } from '../../ui/ModalShell';
import TextField from '../../ui/TextField';
import {
  canUseDirect,
  hasBaseUrl,
  rolesSummary,
  SearchProviderSwatch,
  type Translate,
  withProvider,
} from './searchPanelShared';

/**
 * What the dialog is for:
 * - `enable` — turn a provider on over its direct route (key and/or URL).
 * - `key`    — add or replace the key of a provider that is already on.
 * - `url`    — change a self-hosted provider's instance URL.
 */
export type ConnectMode = 'enable' | 'key' | 'url';

interface Props {
  provider: SearchProviderInfo;
  mode: ConnectMode;
  /** Switch the provider to its direct route as part of saving. */
  switchToDirect?: boolean;
  saving: boolean;
  /** Persist the patch; resolves true when the core accepted it. */
  onSubmit: (patch: SearchProviderUpdate) => Promise<boolean>;
  onClose: () => void;
  t: Translate;
}

const SearchPanelConnectDialog = ({
  provider,
  mode,
  switchToDirect = false,
  saving,
  onSubmit,
  onClose,
  t,
}: Props) => {
  const baseId = useId();
  const showKey = provider.takes_key && mode !== 'url';
  const showUrl = hasBaseUrl(provider) && mode !== 'key';
  const [key, setKey] = useState('');
  const [url, setUrl] = useState(provider.base_url ?? '');

  // A stored key may be kept when only the route or URL changes; a new
  // provider without one must get one before it can serve anything, unless
  // its key is optional.
  const keyRequired = showKey && !provider.key_configured && !provider.key_optional;
  const canSubmit =
    !saving &&
    (!keyRequired || key.trim().length > 0) &&
    (!showUrl || url.trim().length > 0) &&
    (mode !== 'key' || key.trim().length > 0);

  const submit = async () => {
    if (!canSubmit) return;
    const patch: SearchProviderUpdate = {};
    if (mode === 'enable') patch.enabled = true;
    if (
      (mode === 'enable' || switchToDirect) &&
      canUseDirect(provider) &&
      provider.routes.length > 1
    )
      patch.route = 'direct';
    if (showKey && key.trim()) patch.api_key = key.trim();
    if (showUrl) patch.base_url = url.trim();
    if (await onSubmit(patch)) onClose();
  };

  const title =
    mode === 'url'
      ? t('settings.search.baseUrlLabel')
      : withProvider(t('settings.search.connectTitle'), provider.label);

  return (
    <ModalShell
      title={title}
      titleId={`${baseId}-title`}
      subtitle={rolesSummary(provider, t)}
      icon={<SearchProviderSwatch id={provider.id} label={provider.label} />}
      onClose={onClose}
      maxWidthClassName="max-w-md"
      testId={`search-connect-${provider.id}`}
      footer={
        <div className="flex justify-end gap-2">
          <Button type="button" variant="secondary" size="sm" onClick={onClose} disabled={saving}>
            {t('common.cancel')}
          </Button>
          <Button
            type="button"
            variant="primary"
            size="sm"
            data-testid={`search-connect-${provider.id}-submit`}
            disabled={!canSubmit}
            onClick={() => void submit()}>
            {mode === 'enable' ? t('settings.search.connect') : t('common.save')}
          </Button>
        </div>
      }>
      <form
        className="flex flex-col gap-4"
        onSubmit={event => {
          event.preventDefault();
          void submit();
        }}>
        {showKey && (
          <div className="flex flex-col gap-1.5">
            <div className="flex items-center justify-between gap-2">
              <Label htmlFor={`${baseId}-key`} className="text-xs text-content-secondary">
                {withProvider(t('settings.search.apiKeyLabel'), provider.label)}
              </Label>
              {provider.docs_url && (
                <a
                  href={provider.docs_url}
                  target="_blank"
                  rel="noopener noreferrer"
                  onClick={event => {
                    event.preventDefault();
                    const target = provider.docs_url;
                    if (target) void openUrl(target).catch(() => undefined);
                  }}
                  className="inline-flex items-center gap-1 text-xs font-medium text-primary-600 hover:underline dark:text-primary-300">
                  {t('settings.search.getApiKey')}
                  <ExternalLink className="h-3 w-3" aria-hidden />
                </a>
              )}
            </div>
            <TextField
              id={`${baseId}-key`}
              data-testid={`search-connect-${provider.id}-key`}
              type="password"
              mono
              autoFocus
              autoComplete="off"
              spellCheck={false}
              data-lpignore="true"
              data-1p-ignore="true"
              value={key}
              disabled={saving}
              placeholder={
                provider.key_configured
                  ? t('settings.search.placeholderStored')
                  : withProvider(t('settings.search.placeholderKey'), provider.label)
              }
              onChange={e => setKey(e.target.value)}
            />
            {provider.deep_research_available !== undefined && (
              <p className="text-[11px] leading-4 text-content-muted">
                {withProvider(t('settings.search.deepResearchHint'), provider.label)}
              </p>
            )}
            {provider.key_optional && (
              <p
                className="text-[11px] leading-4 text-content-muted"
                data-testid={`search-connect-${provider.id}-key-optional`}>
                {withProvider(t('settings.search.optionalKeyHint'), provider.label)}
              </p>
            )}
          </div>
        )}

        {showUrl && (
          <div className="flex flex-col gap-1.5">
            <Label htmlFor={`${baseId}-url`} className="text-xs text-content-secondary">
              {t('settings.search.baseUrlLabel')}
            </Label>
            <TextField
              id={`${baseId}-url`}
              data-testid={`search-connect-${provider.id}-url`}
              type="url"
              mono
              autoFocus={!showKey}
              spellCheck={false}
              value={url}
              disabled={saving}
              placeholder="http://localhost:8080"
              onChange={e => setUrl(e.target.value)}
            />
            <p className="text-[11px] leading-4 text-content-muted">
              {t('settings.search.baseUrlHint')}
            </p>
          </div>
        )}
      </form>
    </ModalShell>
  );
};

export default SearchPanelConnectDialog;
