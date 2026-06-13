/**
 * Export tab of the Sharable MCP Inventory panel.
 *
 * Renders the user's current installed MCP servers as a versioned,
 * secret-free manifest in a code block, with Copy and Download
 * actions. A loud privacy banner re-states what is and is not in the
 * manifest so the user sees it before sharing the artifact.
 */
import { useMemo, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  buildManifest,
  type McpInventoryManifest,
  serializeManifest,
  suggestedFilename,
} from './McpInventoryManifest';
import type { InstalledServer } from './types';

interface McpInventoryExportTabProps {
  servers: InstalledServer[];
}

const McpInventoryExportTab = ({ servers }: McpInventoryExportTabProps) => {
  const { t } = useT();
  const [copyStatus, setCopyStatus] = useState<'idle' | 'copied'>('idle');

  const manifest: McpInventoryManifest = useMemo(() => buildManifest(servers), [servers]);
  const serialized = useMemo(() => serializeManifest(manifest), [manifest]);

  if (servers.length === 0) {
    return (
      <p className="text-sm text-stone-500 dark:text-neutral-400 py-6 text-center">
        {t('mcp.inventory.export.empty')}
      </p>
    );
  }

  const handleCopy = async () => {
    if (typeof navigator === 'undefined' || !navigator.clipboard) return;
    try {
      await navigator.clipboard.writeText(serialized);
      setCopyStatus('copied');
      window.setTimeout(() => setCopyStatus('idle'), 1500);
    } catch {
      // Silently no-op on platforms / contexts without clipboard access.
      // The serialized text is visible and selectable in the <pre>.
    }
  };

  const handleDownload = () => {
    if (typeof document === 'undefined') return;
    const blob = new Blob([serialized], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const link = document.createElement('a');
    link.href = url;
    link.download = suggestedFilename(manifest);
    document.body.appendChild(link);
    link.click();
    document.body.removeChild(link);
    // Revoke after the click handler is done so the browser has a
    // chance to start the download.
    setTimeout(() => URL.revokeObjectURL(url), 0);
  };

  return (
    <div className="space-y-3">
      <div
        role="note"
        className="rounded-lg border border-sage-200 dark:border-sage-500/30 bg-sage-50 dark:bg-sage-500/10 px-3 py-2 text-xs text-stone-700 dark:text-neutral-200">
        <p className="font-medium mb-1">{t('mcp.inventory.export.privacyTitle')}</p>
        <p>{t('mcp.inventory.export.privacyBody')}</p>
      </div>
      <div className="flex items-center justify-between gap-2">
        <p className="text-xs text-stone-500 dark:text-neutral-400">
          {t('mcp.inventory.export.serverCount').replace('{count}', String(servers.length))}
        </p>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => void handleCopy()}
            aria-label={t('mcp.inventory.export.copyAria')}
            className="rounded-lg border border-stone-200 dark:border-neutral-700 px-3 py-1.5 text-xs font-medium text-stone-700 dark:text-neutral-200 hover:bg-stone-50 dark:hover:bg-neutral-800">
            {copyStatus === 'copied'
              ? t('mcp.inventory.export.copied')
              : t('mcp.inventory.export.copy')}
          </button>
          <button
            type="button"
            onClick={handleDownload}
            aria-label={t('mcp.inventory.export.downloadAria')}
            className="rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-primary-600">
            {t('mcp.inventory.export.download')}
          </button>
        </div>
      </div>
      <pre
        data-testid="mcp-inventory-export-pre"
        className="max-h-80 overflow-auto rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-950 p-3 text-[11px] font-mono text-stone-800 dark:text-neutral-100 whitespace-pre-wrap break-words">
        {serialized}
      </pre>
    </div>
  );
};

export default McpInventoryExportTab;
