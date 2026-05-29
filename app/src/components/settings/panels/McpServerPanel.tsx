import debug from 'debug';
import { useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
// `safeInvoke` (aliased to `invoke`) converts the CEF
// `window.ipc.postMessage` synchronous throw — Sentry TAURI-REACT-7 /
// TAURI-REACT-6 — into a rejected Promise that the existing try/catch sees
// as a regular IPC failure.
import { safeInvoke as invoke, isTauri } from '../../../utils/tauriCommands/common';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

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
  const { navigateBack, breadcrumbs } = useSettingsNavigation();

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

  return (
    <div className="z-10 relative">
      {!embedded && (
        <SettingsHeader
          title={t('settings.mcpServer.title')}
          showBackButton={true}
          onBack={navigateBack}
          breadcrumbs={breadcrumbs}
        />
      )}

      {/* ----------------------------------------------------------------- */}
      {/* Section 1 — Available Tools                                        */}
      {/* ----------------------------------------------------------------- */}
      <div className="px-4 pt-4 pb-2">
        <div className="text-sm font-semibold text-slate-900 dark:text-neutral-100 mb-0.5">
          {t('settings.mcpServer.toolsSectionTitle')}
        </div>
        <div className="text-xs text-slate-500 dark:text-neutral-400 mb-3">
          {t('settings.mcpServer.toolsSectionDesc')}
        </div>
        <div className="rounded-xl border border-stone-200 dark:border-neutral-800 divide-y divide-stone-100 dark:divide-neutral-800 overflow-hidden">
          {MCP_TOOLS.map(tool => (
            <div
              key={tool.name}
              className="flex items-start gap-3 px-4 py-2.5 bg-white dark:bg-neutral-900">
              <span className="font-mono text-xs text-primary-700 dark:text-primary-400 mt-0.5 shrink-0">
                {tool.name}
              </span>
              <span className="text-xs text-slate-600 dark:text-neutral-400">
                {tool.description}
              </span>
            </div>
          ))}
        </div>
      </div>

      {/* ----------------------------------------------------------------- */}
      {/* Section 2 — Client Configuration                                   */}
      {/* ----------------------------------------------------------------- */}
      <div className="px-4 pt-4 pb-6">
        <div className="text-sm font-semibold text-slate-900 dark:text-neutral-100 mb-0.5">
          {t('settings.mcpServer.configSectionTitle')}
        </div>
        <div className="text-xs text-slate-500 dark:text-neutral-400 mb-3">
          {t('settings.mcpServer.configSectionDesc')}
        </div>

        {/* Client selector tabs */}
        <div
          className="flex gap-1 mb-4 flex-wrap"
          role="tablist"
          aria-label={t('settings.mcpServer.clientSelectorAriaLabel')}>
          {clients.map(client => (
            <button
              key={client.id}
              role="tab"
              aria-selected={activeClient === client.id}
              onClick={() => {
                setActiveClient(client.id);
                setOpenConfigError(null);
              }}
              className={[
                'px-3 py-1.5 rounded-lg text-xs font-medium transition-colors',
                activeClient === client.id
                  ? 'bg-primary-600 text-white'
                  : 'bg-stone-100 dark:bg-neutral-800 text-slate-700 dark:text-neutral-300 hover:bg-stone-200 dark:hover:bg-neutral-700',
              ].join(' ')}>
              {client.label}
            </button>
          ))}
        </div>

        {/* Binary path error banner */}
        {binaryError && (
          <div className="mb-3 px-3 py-2 rounded-lg border border-coral-300 dark:border-coral-500/40 bg-coral-50 dark:bg-coral-500/10 text-xs text-coral-900 dark:text-coral-300">
            {t('settings.mcpServer.binaryPathNotFound')}
          </div>
        )}

        {/* Config file path */}
        <div className="mb-2 flex items-center gap-2">
          <span className="text-xs text-slate-500 dark:text-neutral-400 shrink-0">
            {t('settings.mcpServer.configFilePath')}:
          </span>
          <span className="text-xs font-mono text-slate-700 dark:text-neutral-300 truncate">
            {configPath}
          </span>
        </div>

        {/* JSON snippet */}
        <div className="rounded-xl overflow-hidden border border-stone-200 dark:border-neutral-800 mb-3">
          <pre className="bg-stone-50 dark:bg-neutral-900/60 px-4 py-3 text-xs font-mono text-slate-800 dark:text-neutral-200 overflow-x-auto whitespace-pre leading-relaxed">
            {snippet}
          </pre>
        </div>

        {/* Action buttons */}
        <div className="flex items-center gap-2 flex-wrap">
          <button
            onClick={handleCopy}
            className="px-3 py-1.5 rounded-lg text-xs font-medium bg-slate-700 hover:bg-slate-600 text-white transition-colors shrink-0">
            {copied ? t('settings.mcpServer.copied') : t('settings.mcpServer.copySnippet')}
          </button>

          {isTauri() && (
            <button
              onClick={handleOpenConfig}
              className="px-3 py-1.5 rounded-lg text-xs font-medium bg-stone-100 dark:bg-neutral-800 text-slate-700 dark:text-neutral-300 hover:bg-stone-200 dark:hover:bg-neutral-700 transition-colors shrink-0">
              {t('settings.mcpServer.openConfigFile')}
            </button>
          )}
        </div>

        {/* Open config error */}
        {openConfigError && (
          <div
            role="status"
            aria-live="polite"
            className="mt-2 text-xs text-coral-600 dark:text-coral-300">
            {t('settings.mcpServer.openConfigError')}: {openConfigError}
          </div>
        )}
      </div>
    </div>
  );
};

export default McpServerPanel;
