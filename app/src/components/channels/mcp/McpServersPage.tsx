/**
 * Connections → MCP Servers: the user's tool servers, said three ways.
 *
 * The page owns its own header — title, description and the tab strip — the
 * way the LLM page does, so the tabs sit in the header's chrome rather than in
 * a card under it. **Servers** is the list, **mcp.json** the same configuration
 * as one document, **Registry** the directories, where a hosted server can be
 * added in one step. The first two are tabs and not two pages because they are
 * not two things: both go through the same core RPCs into the same store.
 */
import { useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import SettingsTabbedPage from '../../settings/layout/SettingsTabbedPage';
import McpServerPanel from '../../settings/panels/McpServerPanel';
import BetaIndicator from '../../ui/BetaIndicator';
import McpServersTab, { type McpPageTab } from './McpServersTab';

/**
 * `clients` sets up external MCP clients (Claude Desktop, Cursor, …) to use
 * OpenHuman as their server. It was the Settings → MCP Server page.
 */
type McpTab = McpPageTab | 'clients';

interface McpServersPageProps {
  initialTab?: McpTab;
}

const McpServersPage = ({ initialTab = 'servers' }: McpServersPageProps) => {
  const { t } = useT();
  const [tab, setTab] = useState<McpTab>(initialTab);

  return (
    <SettingsTabbedPage
      title={t('connections.tabs.mcp')}
      description={t('connections.header.mcp')}
      headerAction={<BetaIndicator />}
      tabs={[
        { id: 'servers', label: t('mcp.tab.section.servers') },
        { id: 'clients', label: t('mcp.tab.section.clients') },
        { id: 'json', label: t('mcp.tab.section.json') },
        { id: 'registry', label: t('mcp.tab.section.registry') },
      ]}
      value={tab}
      onChange={setTab}
      tabsAriaLabel={t('mcp.tab.tablistAria')}
      tabsTestIdPrefix="mcp-page-tab"
      // The Servers table fills the body and scrolls its own rows; the other
      // tabs are documents that scroll with the page.
      scrollable={tab !== 'servers'}>
      {tab === 'clients' ? (
        <McpServerPanel embedded />
      ) : (
        <McpServersTab tab={tab} onTabChange={setTab} />
      )}
    </SettingsTabbedPage>
  );
};

export default McpServersPage;
