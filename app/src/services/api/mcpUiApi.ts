/**
 * Typed RPC wrapper for the `mcp_ui` domain: widget documents and the tool
 * calls a widget asks for.
 */
import debug from 'debug';

import type {
  McpUiResource,
  McpUiToolCallResult,
} from '../../features/conversations/tools/mcpUi/types';
import { callCoreRpc } from '../coreRpcClient';

const log = debug('mcp-ui:api');

export interface ResourceReadParams {
  serverId?: string;
  uri?: string;
  inlineId?: string;
}

export const mcpUiApi = {
  async resourceRead({ serverId, uri, inlineId }: ResourceReadParams): Promise<McpUiResource> {
    log('resource_read server=%s inline=%s', serverId ?? '-', inlineId ? 'yes' : 'no');
    return callCoreRpc<McpUiResource>({
      method: 'openhuman.mcp_ui_resource_read',
      params: { server_id: serverId, uri, inline_id: inlineId },
    });
  },

  async toolCall(
    serverId: string,
    toolName: string,
    args: Record<string, unknown>,
    confirmed: boolean
  ): Promise<McpUiToolCallResult> {
    log('tool_call server=%s tool=%s confirmed=%s', serverId, toolName, confirmed);
    return callCoreRpc<McpUiToolCallResult>({
      method: 'openhuman.mcp_ui_tool_call',
      params: { server_id: serverId, tool_name: toolName, arguments: args, confirmed },
    });
  },
};
