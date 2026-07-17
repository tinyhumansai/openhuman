/**
 * CustomServerFormModal — add / edit a hand-entered MCP server.
 *
 * Reachable from `CustomServersPanel` via its Add button (create) or a server
 * row's Edit button (edit). Covers the servers no registry lists: a local
 * command (`stdio`) or a hosted endpoint (`http_remote`).
 *
 * The core owns validation — this form mirrors the cheap rules to keep the
 * submit button honest, but never assumes it caught everything: submit failures
 * surface the core's message verbatim.
 */
import createDebug from 'debug';
import { useMemo, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import type { CustomServerParams } from '../../../services/api/mcpClientsApi';
import Button from '../../ui/Button';
import { ModalShell } from '../../ui/ModalShell';
import type { InstalledServer } from './types';

const log = createDebug('app:mcp:CustomServerFormModal');

type TransportKind = 'stdio' | 'http_remote';

/** One row of the env / header editor. Kept as a list, not a map, so a
 *  half-typed key doesn't collide with another row while the user types. */
interface EnvRow {
  id: string;
  key: string;
  value: string;
}

export interface CustomServerFormModalProps {
  mode: 'create' | 'edit';
  /** The row being edited. Required when `mode === 'edit'`. */
  server?: InstalledServer;
  onClose: () => void;
  onSubmit: (params: CustomServerParams) => Promise<void>;
}

let rowSeq = 0;
const newRow = (key = '', value = ''): EnvRow => ({ id: `row-${rowSeq++}`, key, value });

/**
 * Keys the core reserves for internal connection state (`__oauth__` holds the
 * OAuth refresh bundle). They appear in `env_keys` but are not the user's to
 * edit, and the core carries them across an update on its own.
 */
const RESERVED_ENV_PREFIX = '__';

/**
 * Seed the env editor from an existing record.
 *
 * Only key *names* come back from the core — values are write-only by design,
 * so every seeded row starts blank. The core reads a blank value as "keep the
 * stored secret" (see `buildEnv`), which is what makes an edit that only
 * renames the server safe.
 */
const seedEnvRows = (server?: InstalledServer): EnvRow[] => {
  const keys = (server?.env_keys ?? []).filter(k => !k.startsWith(RESERVED_ENV_PREFIX));
  return keys.length > 0 ? keys.map(k => newRow(k, '')) : [newRow()];
};

const seedArgs = (server?: InstalledServer): string => (server?.args ?? []).join('\n');

const seedTransport = (server?: InstalledServer): TransportKind =>
  server?.transport?.kind === 'http_remote' ? 'http_remote' : 'stdio';

const seedUrl = (server?: InstalledServer): string =>
  server?.transport?.kind === 'http_remote' ? server.transport.url : '';

const inputClass =
  'w-full rounded-lg border border-line bg-surface px-3 py-2 text-sm text-content ' +
  'placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:outline-none ' +
  'focus:ring-2 focus:ring-primary-500/40';

const labelClass = 'block text-xs font-medium text-content-muted mb-1';

const CustomServerFormModal = ({ mode, server, onClose, onSubmit }: CustomServerFormModalProps) => {
  const { t } = useT();

  const [displayName, setDisplayName] = useState(server?.display_name ?? '');
  const [description, setDescription] = useState(server?.description ?? '');
  const [transport, setTransport] = useState<TransportKind>(() => seedTransport(server));
  const [command, setCommand] = useState(server?.command ?? '');
  const [argsText, setArgsText] = useState(() => seedArgs(server));
  const [url, setUrl] = useState(() => seedUrl(server));
  const [envRows, setEnvRows] = useState<EnvRow[]>(() => seedEnvRows(server));
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const isEdit = mode === 'edit';

  // Mirrors the core's required-field rules so the button reflects them. The
  // core is still the authority — see the submit handler.
  const canSubmit = useMemo(() => {
    if (!displayName.trim()) return false;
    if (transport === 'stdio') return Boolean(command.trim());
    return Boolean(url.trim());
  }, [displayName, transport, command, url]);

  const updateRow = (id: string, patch: Partial<EnvRow>) =>
    setEnvRows(rows => rows.map(r => (r.id === id ? { ...r, ...patch } : r)));

  const removeRow = (id: string) =>
    setEnvRows(rows => (rows.length > 1 ? rows.filter(r => r.id !== id) : [newRow()]));

  /**
   * Collapse the editor rows into the wire map.
   *
   * A keyed row is always sent, blank value included: the core reads a blank as
   * "keep the stored secret" and treats the submitted key *set* as
   * authoritative. Dropping a blank row here would therefore delete the
   * credential rather than preserve it — the whole point of the blank-means-keep
   * rule. Removing a row in the UI is what deletes a key.
   */
  const buildEnv = (): Record<string, string> => {
    const env: Record<string, string> = {};
    for (const row of envRows) {
      const key = row.key.trim();
      if (!key) continue;
      env[key] = row.value;
    }
    return env;
  };

  const handleSubmit = async () => {
    setSaving(true);
    setError(null);
    const params: CustomServerParams = {
      display_name: displayName.trim(),
      transport,
      description: description.trim() || undefined,
      env: buildEnv(),
      ...(transport === 'stdio'
        ? {
            command: command.trim(),
            args: argsText
              .split('\n')
              .map(a => a.trim())
              .filter(Boolean),
          }
        : { url: url.trim() }),
    };
    log('submit mode=%s transport=%s envKeys=%o', mode, transport, Object.keys(params.env ?? {}));
    try {
      await onSubmit(params);
      onClose();
    } catch (err) {
      // Surface the core's message rather than a generic string: it names the
      // offending field (bad scheme, reserved env key, …) and is the only thing
      // that tells the user what to change.
      const msg = err instanceof Error ? err.message : String(err);
      log('submit failed: %s', msg);
      setError(msg);
    } finally {
      setSaving(false);
    }
  };

  const envLabel =
    transport === 'stdio' ? t('mcp.custom.form.envVars') : t('mcp.custom.form.headers');
  const envHint =
    transport === 'stdio' ? t('mcp.custom.form.envHint') : t('mcp.custom.form.headersHint');

  return (
    <ModalShell
      titleId="custom-server-form-title"
      title={isEdit ? t('mcp.custom.form.editTitle') : t('mcp.custom.form.addTitle')}
      subtitle={t('mcp.custom.form.subtitle')}
      maxWidthClassName="max-w-lg"
      contentClassName="px-5 py-4 max-h-[70vh] overflow-y-auto"
      onClose={onClose}>
      <div className="space-y-4" data-testid="custom-server-form">
        <div>
          <label className={labelClass} htmlFor="custom-server-name">
            {t('mcp.custom.form.name')}
          </label>
          <input
            id="custom-server-name"
            className={inputClass}
            value={displayName}
            onChange={e => setDisplayName(e.target.value)}
            placeholder={t('mcp.custom.form.namePlaceholder')}
          />
        </div>

        <div>
          <span className={labelClass}>{t('mcp.custom.form.transport')}</span>
          <div className="flex gap-2" role="radiogroup" aria-label={t('mcp.custom.form.transport')}>
            {(['stdio', 'http_remote'] as const).map(kind => (
              <button
                key={kind}
                type="button"
                role="radio"
                aria-checked={transport === kind}
                onClick={() => setTransport(kind)}
                className={`flex-1 rounded-lg border px-3 py-2 text-sm font-medium transition-colors ${
                  transport === kind
                    ? 'border-primary-500 bg-primary-50 text-primary-700 dark:bg-primary-500/10'
                    : 'border-line text-content-muted hover:bg-surface-muted'
                }`}>
                {kind === 'stdio'
                  ? t('mcp.custom.form.transportLocal')
                  : t('mcp.custom.form.transportRemote')}
              </button>
            ))}
          </div>
          <p className="mt-1 text-xs text-content-muted">
            {transport === 'stdio'
              ? t('mcp.custom.form.transportLocalHint')
              : t('mcp.custom.form.transportRemoteHint')}
          </p>
        </div>

        {transport === 'stdio' ? (
          <>
            <div>
              <label className={labelClass} htmlFor="custom-server-command">
                {t('mcp.custom.form.command')}
              </label>
              <input
                id="custom-server-command"
                className={inputClass}
                value={command}
                onChange={e => setCommand(e.target.value)}
                placeholder={t('mcp.custom.form.commandPlaceholder')}
              />
            </div>
            <div>
              <label className={labelClass} htmlFor="custom-server-args">
                {t('mcp.custom.form.args')}
              </label>
              <textarea
                id="custom-server-args"
                className={`${inputClass} font-mono`}
                rows={3}
                value={argsText}
                onChange={e => setArgsText(e.target.value)}
                placeholder={t('mcp.custom.form.argsPlaceholder')}
              />
              <p className="mt-1 text-xs text-content-muted">{t('mcp.custom.form.argsHint')}</p>
            </div>
          </>
        ) : (
          <div>
            <label className={labelClass} htmlFor="custom-server-url">
              {t('mcp.custom.form.url')}
            </label>
            <input
              id="custom-server-url"
              className={inputClass}
              value={url}
              onChange={e => setUrl(e.target.value)}
              placeholder={t('mcp.custom.form.urlPlaceholder')}
            />
          </div>
        )}

        <div>
          <span className={labelClass}>{envLabel}</span>
          <div className="space-y-2">
            {envRows.map(row => (
              <div key={row.id} className="flex items-center gap-2">
                <input
                  className={`${inputClass} flex-1`}
                  value={row.key}
                  onChange={e => updateRow(row.id, { key: e.target.value })}
                  placeholder={
                    transport === 'stdio'
                      ? t('mcp.custom.form.envKeyPlaceholder')
                      : t('mcp.custom.form.headerKeyPlaceholder')
                  }
                  aria-label={t('mcp.custom.form.envKeyAria')}
                />
                <input
                  className={`${inputClass} flex-1`}
                  type="password"
                  value={row.value}
                  onChange={e => updateRow(row.id, { value: e.target.value })}
                  placeholder={
                    isEdit && row.key
                      ? t('mcp.custom.form.envValueKeepPlaceholder')
                      : t('mcp.custom.form.envValuePlaceholder')
                  }
                  aria-label={t('mcp.custom.form.envValueAria')}
                />
                <Button
                  variant="tertiary"
                  size="sm"
                  onClick={() => removeRow(row.id)}
                  aria-label={t('mcp.custom.form.envRemoveAria')}>
                  ✕
                </Button>
              </div>
            ))}
          </div>
          <div className="mt-2">
            <Button variant="secondary" size="sm" onClick={() => setEnvRows(r => [...r, newRow()])}>
              {t('mcp.custom.form.envAdd')}
            </Button>
          </div>
          <p className="mt-1 text-xs text-content-muted">{envHint}</p>
        </div>

        <div>
          <label className={labelClass} htmlFor="custom-server-description">
            {t('mcp.custom.form.description')}
          </label>
          <input
            id="custom-server-description"
            className={inputClass}
            value={description}
            onChange={e => setDescription(e.target.value)}
            placeholder={t('mcp.custom.form.descriptionPlaceholder')}
          />
        </div>

        {error ? (
          <p
            role="alert"
            className="rounded-lg border border-coral-300 bg-coral-50 px-3 py-2 text-xs text-coral-700 dark:bg-coral-500/10">
            {error}
          </p>
        ) : null}

        <div className="flex justify-end gap-2 border-t border-line-subtle pt-4">
          <Button variant="secondary" size="md" onClick={onClose} disabled={saving}>
            {t('common.cancel')}
          </Button>
          <Button
            variant="primary"
            size="md"
            onClick={() => void handleSubmit()}
            disabled={!canSubmit || saving}>
            {saving
              ? t('mcp.custom.form.saving')
              : isEdit
                ? t('mcp.custom.form.save')
                : t('mcp.custom.form.add')}
          </Button>
        </div>
      </div>
    </ModalShell>
  );
};

export default CustomServerFormModal;
