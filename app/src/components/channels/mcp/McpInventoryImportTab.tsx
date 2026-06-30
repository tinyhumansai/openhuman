/**
 * Import tab of the Sharable MCP Inventory panel.
 *
 * The flow is intentionally THREE-STEP and never auto-installs:
 *
 *   1. **Source** — user pastes JSON or uploads a file.
 *   2. **Preview** — manifest is parsed + validated; each entry is
 *      classified (`new` vs `already_installed`); the validation error,
 *      if any, is surfaced via `role="alert"`.
 *   3. **Per-entry install** — each `new` entry has its own "Install"
 *      button that calls the parent's existing install-dialog flow with
 *      the env_keys pre-filled (as empty values). We deliberately do
 *      NOT collect secret values here — the proven InstallDialog
 *      surface already handles that.
 *
 * No automatic bulk-install was implemented: an MCP server is a piece of
 * trust the user is granting to their agent. A one-click-install-many
 * action would invite supply-chain attacks via malicious manifests. The
 * per-entry "Install" preserves friction at exactly the right step.
 */
import { type ChangeEvent, useCallback, useMemo, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import Button from '../../ui/Button';
import {
  type ClassifiedImportEntry,
  classifyImport,
  type McpInventoryManifest,
  type ParseErrorCode,
  parseManifest,
} from './McpInventoryManifest';
import type { InstalledServer } from './types';

interface McpInventoryImportTabProps {
  installedServers: InstalledServer[];
  onInstallServer: (qualifiedName: string, prefillEnv: Record<string, string>) => void;
}

const McpInventoryImportTab = ({
  installedServers,
  onInstallServer,
}: McpInventoryImportTabProps) => {
  const { t } = useT();
  const [rawInput, setRawInput] = useState('');
  const [manifest, setManifest] = useState<McpInventoryManifest | null>(null);
  const [parseError, setParseError] = useState<string | null>(null);
  const [fileError, setFileError] = useState<string | null>(null);

  const classified: ClassifiedImportEntry[] = useMemo(
    () => (manifest ? classifyImport(manifest, installedServers) : []),
    [manifest, installedServers]
  );

  // Map a stable manifest-layer error code to the user-visible string.
  // Detail (machine context — JSON parse text, offending index, etc.)
  // is appended after a separator when present so the alert text reads
  // as one sentence.
  const renderParseError = useCallback(
    (errorCode: ParseErrorCode, detail?: string): string => {
      const base = t(`mcp.inventory.parseError.${errorCode}`);
      return detail ? `${base} (${detail})` : base;
    },
    [t]
  );

  const handleParse = useCallback(() => {
    setFileError(null);
    const result = parseManifest(rawInput);
    if (result.ok) {
      setManifest(result.manifest);
      setParseError(null);
    } else {
      setManifest(null);
      setParseError(renderParseError(result.errorCode, result.detail));
    }
  }, [rawInput, renderParseError]);

  const handleFileChange = useCallback(
    (event: ChangeEvent<HTMLInputElement>) => {
      const file = event.target.files?.[0];
      // Allow re-selecting the same file by clearing the input value.
      event.target.value = '';
      if (!file) return;
      if (file.size > 1_000_000) {
        // 1 MB is generous for a manifest (~10k servers at 100 bytes each).
        // Reject anything larger as a defence against accidental upload
        // of an unrelated big JSON blob. Clear any stale preview /
        // parse-error state so the rejected upload doesn't leave a
        // previous (and now misleading) preview actionable below.
        setManifest(null);
        setParseError(null);
        setFileError(t('mcp.inventory.import.fileTooLarge'));
        return;
      }
      setFileError(null);
      const reader = new FileReader();
      reader.onload = () => {
        const text = typeof reader.result === 'string' ? reader.result : '';
        setRawInput(text);
        const result = parseManifest(text);
        if (result.ok) {
          setManifest(result.manifest);
          setParseError(null);
        } else {
          setManifest(null);
          setParseError(renderParseError(result.errorCode, result.detail));
        }
      };
      reader.onerror = () => {
        // Same reasoning as the size-reject branch — drop any stale
        // preview so a failed re-upload doesn't leave the previous
        // manifest's preview rows still rendering / actionable.
        setManifest(null);
        setParseError(null);
        setFileError(t('mcp.inventory.import.fileReadFailed'));
      };
      reader.readAsText(file);
    },
    [t, renderParseError]
  );

  const handleClear = useCallback(() => {
    setRawInput('');
    setManifest(null);
    setParseError(null);
    setFileError(null);
  }, []);

  const handleInstall = useCallback(
    (entry: ClassifiedImportEntry['entry']) => {
      // Build the prefill object: keys from the manifest, empty values
      // for the user to fill in the existing InstallDialog. The dialog's
      // existing required-field validation does the rest.
      const prefill: Record<string, string> = {};
      for (const key of entry.env_keys) prefill[key] = '';
      onInstallServer(entry.qualified_name, prefill);
    },
    [onInstallServer]
  );

  const stats = useMemo(() => {
    let newly = 0;
    let already = 0;
    for (const c of classified) {
      if (c.status === 'new') newly += 1;
      else already += 1;
    }
    return { newly, already, total: classified.length };
  }, [classified]);

  return (
    <div className="space-y-3">
      <div
        role="note"
        className="rounded-lg border border-amber-200 dark:border-amber-500/30 bg-amber-50 dark:bg-amber-500/10 px-3 py-2 text-xs text-content-secondary">
        <p className="font-medium mb-1">{t('mcp.inventory.import.trustTitle')}</p>
        <p>{t('mcp.inventory.import.trustBody')}</p>
      </div>

      <div className="space-y-1.5">
        <label
          htmlFor="mcp-inventory-import-textarea"
          className="text-xs font-medium text-content-secondary">
          {t('mcp.inventory.import.pasteLabel')}
        </label>
        <textarea
          id="mcp-inventory-import-textarea"
          value={rawInput}
          onChange={e => {
            setRawInput(e.target.value);
            // Live-clear stale parse errors so the user sees a clean
            // slate as they edit; they'll re-appear on "Preview".
            setParseError(null);
          }}
          spellCheck={false}
          rows={6}
          placeholder={t('mcp.inventory.import.pastePlaceholder')}
          aria-label={t('mcp.inventory.import.pasteLabel')}
          className="w-full rounded-lg border border-line bg-surface px-3 py-2 text-xs font-mono text-content focus:outline-none focus:ring-2 focus:ring-primary-400 focus:border-primary-400 resize-y"
        />
      </div>

      <div className="flex items-center justify-between gap-2 flex-wrap">
        <div className="flex items-center gap-2">
          <Button
            variant="primary"
            size="sm"
            onClick={handleParse}
            disabled={rawInput.trim().length === 0}>
            {t('mcp.inventory.import.preview')}
          </Button>
          {(manifest || parseError || rawInput.length > 0) && (
            <Button variant="secondary" size="sm" onClick={handleClear}>
              {t('mcp.inventory.import.clear')}
            </Button>
          )}
        </div>
        <label className="text-xs text-primary-600 dark:text-primary-300 cursor-pointer hover:underline">
          {t('mcp.inventory.import.uploadFile')}
          <input
            type="file"
            accept="application/json,.json"
            onChange={handleFileChange}
            className="sr-only"
            aria-label={t('mcp.inventory.import.uploadFileAria')}
          />
        </label>
      </div>

      {fileError && (
        <p role="alert" className="text-[11px] text-coral-700 dark:text-coral-300">
          {fileError}
        </p>
      )}
      {parseError && (
        <p role="alert" className="text-[11px] text-coral-700 dark:text-coral-300">
          {t('mcp.inventory.import.parseErrorPrefix')} {parseError}
        </p>
      )}

      {manifest && (
        <section aria-labelledby="mcp-inventory-import-preview-heading" className="space-y-2">
          <h3
            id="mcp-inventory-import-preview-heading"
            className="text-xs font-semibold text-content-secondary">
            {t('mcp.inventory.import.previewHeading')}
          </h3>
          <div role="status" aria-live="polite" className="text-[11px] text-content-muted">
            {t('mcp.inventory.import.previewCounts')
              .replace('{total}', String(stats.total))
              .replace('{newly}', String(stats.newly))
              .replace('{already}', String(stats.already))}
          </div>
          <div className="text-[10px] text-content-faint">
            {t('mcp.inventory.import.exportedFrom').replace('{exporter}', manifest.exported_by)} ·{' '}
            {t('mcp.inventory.import.exportedAt').replace('{when}', manifest.exported_at)}
          </div>
          {classified.length === 0 ? (
            <p className="text-xs text-content-muted">{t('mcp.inventory.import.previewEmpty')}</p>
          ) : (
            <ul className="space-y-1">
              {classified.map(({ entry, status }) => (
                <li
                  key={entry.qualified_name}
                  className="flex items-center justify-between gap-2 rounded-lg border border-line px-3 py-2">
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span
                        className={`inline-flex items-center px-1.5 py-0.5 rounded text-[9px] font-semibold uppercase tracking-wider shrink-0 ${
                          status === 'new'
                            ? 'bg-sage-100 dark:bg-sage-500/20 text-sage-700 dark:text-sage-300'
                            : 'bg-surface-subtle text-content-secondary'
                        }`}>
                        {status === 'new'
                          ? t('mcp.inventory.import.statusNew')
                          : t('mcp.inventory.import.statusAlreadyInstalled')}
                      </span>
                      <span className="text-sm font-medium text-content truncate">
                        {entry.display_name}
                      </span>
                    </div>
                    <p className="text-[11px] font-mono text-content-faint truncate">
                      {entry.qualified_name}
                    </p>
                    {entry.env_keys.length > 0 && (
                      <p className="text-[10px] text-content-muted mt-0.5">
                        {t('mcp.inventory.import.envKeysLabel')}: {entry.env_keys.join(', ')}
                      </p>
                    )}
                  </div>
                  {status === 'new' ? (
                    <Button
                      variant="primary"
                      size="xs"
                      onClick={() => handleInstall(entry)}
                      aria-label={t('mcp.inventory.import.installAria').replace(
                        '{name}',
                        entry.display_name
                      )}
                      className="shrink-0">
                      {t('mcp.inventory.import.install')}
                    </Button>
                  ) : (
                    <span className="shrink-0 text-[10px] text-content-faint italic">
                      {t('mcp.inventory.import.skipped')}
                    </span>
                  )}
                </li>
              ))}
            </ul>
          )}
        </section>
      )}
    </div>
  );
};

export default McpInventoryImportTab;
