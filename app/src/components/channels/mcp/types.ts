/**
 * Shared TypeScript types for the MCP Servers tab.
 * Single source of truth — import from here, not from the API layer.
 */

export type SmitheryServer = {
  qualified_name: string;
  display_name: string;
  description?: string;
  icon_url?: string;
  use_count?: number;
  is_deployed?: boolean;
};

export type SmitheryConnection = {
  type: 'stdio' | 'http';
  deployment_url?: string;
  config_schema?: unknown;
  example_config?: unknown;
  published?: boolean;
};

export type SmitheryServerDetail = SmitheryServer & {
  connections: SmitheryConnection[];
  required_env_keys?: string[];
};

export type CommandKind = 'node' | 'python' | 'binary';

export type InstalledServer = {
  server_id: string;
  qualified_name: string;
  display_name: string;
  description?: string;
  icon_url?: string;
  command_kind: CommandKind;
  command: string;
  args: string[];
  env_keys: string[];
  config?: unknown;
  installed_at: number;
  last_connected_at?: number;
};

export type McpTool = { name: string; description?: string; input_schema: unknown };

export type ServerStatus = 'disconnected' | 'connecting' | 'connected' | 'error';

export type ConnStatus = {
  server_id: string;
  qualified_name: string;
  display_name: string;
  status: ServerStatus;
  tool_count: number;
  last_error?: string;
};
