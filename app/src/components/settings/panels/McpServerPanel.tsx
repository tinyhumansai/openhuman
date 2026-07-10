import debug from 'debug';
import { useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
// `safeInvoke` (aliased to `invoke`) converts the CEF
// `window.ipc.postMessage` synchronous throw — Sentry TAURI-REACT-7 /
// TAURI-REACT-6 — into a rejected Promise that the existing try/catch sees
// as a regular IPC failure.
import { safeInvoke as invoke, isTauri } from '../../../utils/tauriCommands/common';
import ChipTabs from '../../layout/ChipTabs';
import PanelPage from '../../layout/PanelPage';
import Button from '../../ui/Button';
import { SettingsSection } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

const log = debug('mcp-server-panel');

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface McpBinaryInfo {
  path: string;
  os: string;
}

type McpClient = 'claude-desktop' | 'cursor' | 'codex' | 'zed';

// ---------------------------------------------------------------------------
// Static tool catalogue
// ---------------------------------------------------------------------------

const MCP_TOOLS: { name: string; description: string }[] = [
  { name: 'core.list_tools', description: 'List all available MCP tools' },
  { name: 'core.tool_instructions', description: 'Get usage instructions for a tool' },
  { name: 'agent.list_subagents', description: 'List available subagents' },
  { name: 'agent.run_subagent', description: 'Run a subagent with a prompt' },
  { name: 'memory.search', description: 'Search memory by semantic query' },
  { name: 'memory.recall', description: 'Recall specific memories by ID' },
  { name: 'tree.read_chunk', description: 'Read a memory tree chunk' },
  { name: 'tree.browse', description: 'Browse the memory tree structure' },
  { name: 'tree.top_entities', description: 'Get top entities from memory tree' },
  { name: 'tree.list_sources', description: 'List memory tree sources' },
];

// ---------------------------------------------------------------------------
// Config path helpers (mirrored from Rust for display only)
// ---------------------------------------------------------------------------

function configFilePathFor(client: McpClient, os: string): string {
  const isWindows = os === 'windows';
  const isMac = os === 'macos';

  switch (client) {
    case 'claude-desktop':
      if (isMac) return '~/Library/Application Support/Claude/claude_desktop_config.json';
      if (isWindows) return '%APPDATA%\\Claude\\claude_desktop_config.json';
      return '~/.config/Claude/claude_desktop_config.json';
    case 'cursor':
      if (isWindows) return '%USERPROFILE%\\.cursor\\mcp.json';
      return '~/.cursor/mcp.json';
    case 'codex':
      return '~/.codex/config.json';
    case 'zed':
      if (isMac) return '~/Library/Application Support/Zed/settings.json';
      if (isWindows) return '%APPDATA%\\Zed\\settings.json';
      return '~/.config/zed/settings.json';
  }
}

// ---------------------------------------------------------------------------
// JSON snippet builders
// ---------------------------------------------------------------------------

function buildSnippet(client: McpClient, binaryPath: string): string {
  if (client === 'zed') {
    return JSON.stringify(
      { context_servers: { openhuman: { command: { path: binaryPath, args: ['mcp'] } } } },
      null,
      2
    );
  }

  // Claude Desktop, Cursor, Codex
  return JSON.stringify(
    { mcpServers: { openhuman: { command: binaryPath, args: ['mcp'] } } },
    null,
    2
  );
}

// ---------------------------------------------------------------------------
// McpServerPanel component
// ---------------------------------------------------------------------------

interface McpServerPanelProps {
  /** When true, skips the SettingsHeader/back-button affordances so the
   *  panel can be embedded in non-settings surfaces (e.g. the Connections
   *  page MCP Clients tab). */
  embedded?: boolean;
}

const McpServerPanel = ({ embedded = false }: McpServerPanelProps = {}) => {
  const { t } = useT();

  const [binaryInfo, setBinaryInfo] = useState<McpBinaryInfo | null>(null);
  const [binaryError, setBinaryError] = useState<string | null>(null);
  const [activeClient, setActiveClient] = useState<McpClient>('claude-desktop');
  const [copied, setCopied] = useState(false);
  const [openConfigError, setOpenConfigError] = useState<string | null>(null);

  // Resolve the binary path on mount.
  useEffect(() => {
    log('resolving mcp binary path');
    invoke<McpBinaryInfo>('mcp_resolve_binary_path')
      .then(info => {
        log('mcp binary resolved: %s os: %s', info.path, info.os);
        setBinaryInfo(info);
        setBinaryError(null);
      })
      .catch(err => {
        const msg = err instanceof Error ? err.message : String(err);
        log('mcp binary resolution failed: %s', msg);
        setBinaryError(msg);
        setBinaryInfo(null);
      });
  }, []);

  const binaryPath = binaryInfo?.path ?? null;
  // When binary resolution fails, fall back to navigator.userAgent so Windows/Linux
  // users see the correct config file path instead of the macOS default.
  const os =
    binaryInfo?.os ??
    (/win/i.test(navigator.userAgent) && !/mac/i.test(navigator.userAgent)
      ? 'windows'
      : /linux/i.test(navigator.userAgent)
        ? 'linux'
        : 'macos');
  const displayPath = binaryPath ?? t('settings.mcpServer.binaryPathNotFound');
  const snippet = buildSnippet(activeClient, displayPath);
  const configPath = configFilePathFor(activeClient, os);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(snippet);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // Clipboard write failed — silently ignore.
    }
  };

  const handleOpenConfig = async () => {
    setOpenConfigError(null);
    try {
      await invoke('mcp_open_client_config', { client: activeClient });
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setOpenConfigError(msg);
    }
  };

  const clients: { id: McpClient; label: string }[] = [
    { id: 'claude-desktop', label: t('settings.mcpServer.clientClaudeDesktop') },
    { id: 'cursor', label: t('settings.mcpServer.clientCursor') },
    { id: 'codex', label: t('settings.mcpServer.clientCodex') },
    { id: 'zed', label: t('settings.mcpServer.clientZed') },
  ];

  const body = (
    <>
      {/* ----------------------------------------------------------------- */}
      {/* Section 1 — Available Tools                                        */}
      {/* ----------------------------------------------------------------- */}
      <div className="px-4 pt-4 pb-2">
        <SettingsSection
          title={t('settings.mcpServer.toolsSectionTitle')}
          description={t('settings.mcpServer.toolsSectionDesc')}>
          {MCP_TOOLS.map(tool => (
            <div key={tool.name} className="flex items-start gap-3 px-4 py-3 bg-surface">
              <span className="font-mono text-xs text-primary-700 dark:text-primary-400 mt-0.5 shrink-0">
                {tool.name}
              </span>
              <span className="text-xs text-content-secondary dark:text-content-muted">
                {tool.description}
              </span>
            </div>
          ))}
        </SettingsSection>
      </div>

      {/* ----------------------------------------------------------------- */}
      {/* Section 2 — Client Configuration                                   */}
      {/* ----------------------------------------------------------------- */}
      <div className="px-4 pt-4 pb-6">
        <SettingsSection
          title={t('settings.mcpServer.configSectionTitle')}
          description={t('settings.mcpServer.configSectionDesc')}>
          {/* Client selector tabs */}
          <ChipTabs
            ariaLabel={t('settings.mcpServer.clientSelectorAriaLabel')}
            items={clients}
            value={activeClient}
            onChange={id => {
              setActiveClient(id);
              setOpenConfigError(null);
            }}
          />

          {/* Binary path error banner */}
          {binaryError && (
            <div className="mx-4 mt-3 px-3 py-2 rounded-lg border border-coral-300 dark:border-coral-500/40 bg-coral-50 dark:bg-coral-500/10 text-xs text-coral-900 dark:text-coral-300">
              {t('settings.mcpServer.binaryPathNotFound')}
            </div>
          )}

          {/* Config file path */}
          <div className="px-4 mt-3 mb-2 flex items-center gap-2">
            <span className="text-xs text-content-muted shrink-0">
              {t('settings.mcpServer.configFilePath')}:
            </span>
            <span className="text-xs font-mono text-content-secondary truncate">{configPath}</span>
          </div>

          {/* JSON snippet */}
          <div className="mx-4 mb-3 rounded-xl overflow-hidden border border-line">
            <pre className="bg-surface-muted dark:bg-surface/60 px-4 py-3 text-xs font-mono text-content overflow-x-auto whitespace-pre leading-relaxed">
              {snippet}
            </pre>
          </div>

          {/* Action buttons */}
          <div className="px-4 pb-4 flex items-center gap-2 flex-wrap">
            <Button type="button" variant="secondary" size="xs" onClick={() => void handleCopy()}>
              {copied ? t('settings.mcpServer.copied') : t('settings.mcpServer.copySnippet')}
            </Button>

            {isTauri() && (
              <Button
                type="button"
                variant="tertiary"
                size="xs"
                onClick={() => void handleOpenConfig()}>
                {t('settings.mcpServer.openConfigFile')}
              </Button>
            )}
          </div>

          {/* Open config error */}
          {openConfigError && (
            <div
              role="status"
              aria-live="polite"
              className="px-4 pb-3 text-xs text-coral-600 dark:text-coral-300">
              {t('settings.mcpServer.openConfigError')}: {openConfigError}
            </div>
          )}
        </SettingsSection>
      </div>
    </>
  );

  if (embedded) {
    return (
      <PanelPage className="z-10" contentClassName="">
        {body}
      </PanelPage>
    );
  }

  return (
    <SettingsPanel description={t('settings.developerMenu.mcpServer.desc')}>{body}</SettingsPanel>
  );
};

export default McpServerPanel;
