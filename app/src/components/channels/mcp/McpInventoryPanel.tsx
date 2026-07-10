/**
 * Sharable MCP Inventory — top-level modal hosting Export / Import tabs.
 *
 * The parent (`McpServersTab`) holds the open/close state and the
 * current `servers` array; this component owns the tab navigation and
 * dispatches the install-via-existing-dialog flow back upward.
 *
 * Why a single modal with tabs (rather than two separate modals):
 *   - The user often flips between "let me see what I have" (Export)
 *     and "let me apply what someone sent" (Import) in the same
 *     session — tabbing is faster than re-opening.
 *   - The dialog focus contract (`role="dialog" aria-modal`) is
 *     simpler to maintain on a single mount.
 *
 * Esc closes the modal; backdrop mousedown closes; clicks inside the
 * card do not.
 */
import { useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import Button from '../../ui/Button';
import McpInventoryExportTab from './McpInventoryExportTab';
import McpInventoryImportTab from './McpInventoryImportTab';
import type { InstalledServer } from './types';

interface McpInventoryPanelProps {
  /** Current installed servers — drives the Export tab and the
   *  "already installed" detection in the Import tab. */
  servers: InstalledServer[];
  /**
   * Called when the user clicks "Install" on an entry in the Import
   * preview. Parent wires this to its existing install-dialog flow
   * (`setRightPane({ mode: 'install', qualifiedName, prefillEnv })`)
   * so the proven InstallDialog handles env-value collection — we
   * never re-implement that critical surface here.
   */
  onInstallServer: (qualifiedName: string, prefillEnv: Record<string, string>) => void;
  onClose: () => void;
}

type Tab = 'export' | 'import';

const McpInventoryPanel = ({ servers, onInstallServer, onClose }: McpInventoryPanelProps) => {
  const { t } = useT();
  const [tab, setTab] = useState<Tab>('export');

  // Esc to close, regardless of which child has focus.
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        onClose();
      }
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [onClose]);

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="mcp-inventory-panel-title"
      onMouseDown={event => {
        if (event.target === event.currentTarget) onClose();
      }}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 px-4 py-6 overflow-y-auto">
      <div className="bg-surface rounded-xl shadow-xl max-w-3xl w-full p-5 max-h-full overflow-y-auto">
        <div className="flex items-start justify-between gap-3 mb-3">
          <div>
            <h2 id="mcp-inventory-panel-title" className="text-base font-semibold text-content">
              {t('mcp.inventory.title')}
            </h2>
            <p className="text-xs text-content-muted mt-1">{t('mcp.inventory.subtitle')}</p>
          </div>
          <Button
            iconOnly
            variant="tertiary"
            size="sm"
            onClick={onClose}
            aria-label={t('mcp.inventory.close')}
            className="shrink-0">
            <svg
              className="w-4 h-4"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              strokeWidth={2}
              aria-hidden="true">
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </Button>
        </div>

        {/* Tab bar */}
        <div
          role="tablist"
          aria-label={t('mcp.inventory.tablistAria')}
          className="flex gap-1 border-b border-line mb-4">
          <button
            type="button"
            role="tab"
            id="mcp-inventory-tab-export"
            aria-selected={tab === 'export'}
            aria-controls="mcp-inventory-panel-export"
            tabIndex={tab === 'export' ? 0 : -1}
            onClick={() => setTab('export')}
            className={`-mb-px px-3 py-1.5 text-xs font-medium border-b-2 transition-colors ${
              tab === 'export'
                ? 'border-primary-500 text-primary-600 dark:text-primary-300'
                : 'border-transparent text-content-muted hover:text-content dark:hover:text-neutral-200'
            }`}>
            {t('mcp.inventory.tab.export')}
          </button>
          <button
            type="button"
            role="tab"
            id="mcp-inventory-tab-import"
            aria-selected={tab === 'import'}
            aria-controls="mcp-inventory-panel-import"
            tabIndex={tab === 'import' ? 0 : -1}
            onClick={() => setTab('import')}
            className={`-mb-px px-3 py-1.5 text-xs font-medium border-b-2 transition-colors ${
              tab === 'import'
                ? 'border-primary-500 text-primary-600 dark:text-primary-300'
                : 'border-transparent text-content-muted hover:text-content dark:hover:text-neutral-200'
            }`}>
            {t('mcp.inventory.tab.import')}
          </button>
        </div>

        {tab === 'export' && (
          <div
            role="tabpanel"
            id="mcp-inventory-panel-export"
            aria-labelledby="mcp-inventory-tab-export">
            <McpInventoryExportTab servers={servers} />
          </div>
        )}
        {tab === 'import' && (
          <div
            role="tabpanel"
            id="mcp-inventory-panel-import"
            aria-labelledby="mcp-inventory-tab-import">
            <McpInventoryImportTab
              installedServers={servers}
              onInstallServer={(qualifiedName, prefillEnv) => {
                // The parent's install flow lives outside this modal
                // — close the inventory panel so the InstallDialog has
                // room to render in the main right pane.
                onInstallServer(qualifiedName, prefillEnv);
                onClose();
              }}
            />
          </div>
        )}
      </div>
    </div>
  );
};

export default McpInventoryPanel;
