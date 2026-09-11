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
import { useId, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import ChipTabs from '../../layout/ChipTabs';
import { ModalShell } from '../../ui/ModalShell';
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
  const titleId = useId();

  return (
    <ModalShell
      onClose={onClose}
      titleId={titleId}
      title={t('mcp.inventory.title')}
      subtitle={t('mcp.inventory.subtitle')}
      maxWidthClassName="max-w-3xl"
      contentClassName="max-h-full overflow-y-auto p-5">
      {/* `ChipTabs` brings its own `TabsRoot` (as `display: contents`), so the
          panels cannot be Radix `TabsContent` — that would nest a second Tabs
          root and orphan them. A plain conditional is what the other ChipTabs
          hosts in this area already do. The row keeps `role="tab"` /
          `aria-selected` and Radix's roving focus; only the underline paint
          becomes the app's chip grammar. */}
      <ChipTabs<Tab>
        as="tab"
        ariaLabel={t('mcp.inventory.tablistAria')}
        className="mb-4 flex flex-wrap gap-1.5"
        items={[
          { id: 'export', label: t('mcp.inventory.tab.export') },
          { id: 'import', label: t('mcp.inventory.tab.import') },
        ]}
        value={tab}
        onChange={setTab}
      />

      {tab === 'export' ? (
        <McpInventoryExportTab servers={servers} />
      ) : (
        <McpInventoryImportTab
          installedServers={servers}
          onInstallServer={(qualifiedName, prefillEnv) => {
            // The parent's install flow lives outside this modal — close
            // the inventory panel so the InstallDialog has room to render
            // in the main right pane.
            onInstallServer(qualifiedName, prefillEnv);
            onClose();
          }}
        />
      )}
    </ModalShell>
  );
};

export default McpInventoryPanel;
