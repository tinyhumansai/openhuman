/**
 * Telegram MCP Server
 * Provides tools for interacting with Telegram via MCP protocol
 */

import type { Socket } from "socket.io-client";
import type { TelegramState } from "../../../store/telegram/types";
import type {
  MCPServerConfig,
  MCPTool,
  MCPToolCall,
  MCPToolResult,
} from "../types";

import { store } from "../../../store";
import { selectTelegramUserState } from "../../../store/telegramSelectors";
import { ErrorCategory, logAndFormatError } from "../errorHandler";
import { ValidationError } from "../validation";
import { SocketIOMCPTransportImpl } from "../transport";
import { mcpLog } from "../logger";
import { enforceRateLimit, resetRequestCallCount } from "../rateLimiter";
import {
  useExtraToolDefinition,
  executeUseExtraTool,
  executeExtraToolIfExists,
  getAllExtraTools,
  isExtraToolByName,
} from "../skills";
import {
  type TelegramMCPToolHandler,
  type TelegramMCPToolName,
  TELEGRAM_MCP_TOOL_NAMES,
} from "./types";
import * as tools from "./tools";

export class TelegramMCPServer {
  private transport: SocketIOMCPTransportImpl;
  private config: MCPServerConfig;

  constructor(socket: Socket | null | undefined) {
    this.transport = new SocketIOMCPTransportImpl(socket ?? undefined);
    this.config = { name: "telegram-mcp", version: "1.0.0" };
    mcpLog(`Telegram MCP ${this.config.name} v${this.config.version} ready`);
    this.setupHandlers();
  }

  updateSocket(socket: Socket | null | undefined): void {
    this.transport.updateSocket(socket ?? undefined);
  }

  private setupHandlers(): void {
    this.transport.on("toolCall", (data: unknown) => {
      void this.handleToolCallRequest(
        data as { requestId: string; toolCall: MCPToolCall },
      );
    });

    this.transport.on("listTools", (data: unknown) => {
      const { requestId } = data as { requestId: string };
      try {
        const toolsList = this.listTools();
        this.transport.emit("listToolsResponse", {
          requestId,
          tools: toolsList,
        });
      } catch (error) {
        console.error("[MCP] Failed to list tools", error);
        this.transport.emit("listToolsResponse", { requestId, tools: [] });
      }
    });
  }

  private async handleToolCallRequest(data: {
    requestId: string;
    toolCall: MCPToolCall;
  }): Promise<void> {
    const { requestId, toolCall } = data;
    try {
      // Reset per-request counter at the start of each new request
      resetRequestCallCount();
      const result = await this.handleToolCall(toolCall);
      this.transport.emit("toolResult", { requestId, result });
    } catch (error) {
      const errorResult = logAndFormatError(
        "handleToolCall",
        error instanceof Error ? error : new Error(String(error)),
      );
      this.transport.emit("toolResult", { requestId, result: errorResult });
    }
  }

  private listTools(): MCPTool[] {
    const baseTelegramTools = Object.values(tools).filter(
      (t): t is MCPTool => {
        return (
          t !== undefined &&
          typeof t === "object" &&
          "name" in t &&
          "description" in t &&
          "inputSchema" in t &&
          typeof (t as MCPTool).name === "string" &&
          typeof (t as MCPTool).description === "string"
        );
      },
    );

    // Add the use_extra_tool meta-tool + all extra tool definitions
    const extraTools = getAllExtraTools();

    // Dedupe by name (base tools take precedence)
    const toolsByName = new Map<string, MCPTool>();
    for (const t of baseTelegramTools) toolsByName.set(t.name, t);
    toolsByName.set(useExtraToolDefinition.name, useExtraToolDefinition);
    for (const t of extraTools) {
      if (!toolsByName.has(t.name)) toolsByName.set(t.name, t);
    }

    return Array.from(toolsByName.values());
  }

  private async handleToolCall(toolCall: MCPToolCall): Promise<MCPToolResult> {
    const { name, arguments: args } = toolCall;
    mcpLog(`Executing tool: ${name}`, args);

    // Handle use_extra_tool meta-tool (no rate limit needed)
    if (name === "use_extra_tool") {
      return executeUseExtraTool(args);
    }

    // Check if this is an extra tool from a loaded skill
    if (isExtraToolByName(name)) {
      // Enforce rate limits for extra tools too
      try {
        await enforceRateLimit(name);
      } catch (rateLimitError) {
        return {
          content: [
            {
              type: "text",
              text:
                rateLimitError instanceof Error
                  ? rateLimitError.message
                  : String(rateLimitError),
            },
          ],
          isError: true,
        };
      }

      const result = await executeExtraToolIfExists(name, args);
      if (result) return result;
    }

    // Standard Telegram tool
    const toolHandler = this.findToolHandler(name);
    if (!toolHandler) {
      return {
        content: [{ type: "text", text: `Tool '${name}' not found` }],
        isError: true,
      };
    }

    // Enforce rate limits before executing (may sleep or throw)
    try {
      await enforceRateLimit(name);
    } catch (rateLimitError) {
      return {
        content: [
          {
            type: "text",
            text:
              rateLimitError instanceof Error
                ? rateLimitError.message
                : String(rateLimitError),
          },
        ],
        isError: true,
      };
    }

    const telegramState: TelegramState =
      selectTelegramUserState(store.getState());

    try {
      return await toolHandler(args, {
        telegramState,
        transport: this.transport,
      });
    } catch (error) {
      if (error instanceof ValidationError) {
        return logAndFormatError(name, error, ErrorCategory.VALIDATION);
      }
      return logAndFormatError(
        name,
        error instanceof Error ? error : new Error(String(error)),
      );
    }
  }

  private findToolHandler(name: string): TelegramMCPToolHandler | undefined {
    const isToolName = (n: string): n is TelegramMCPToolName =>
      (TELEGRAM_MCP_TOOL_NAMES as readonly string[]).includes(n);

    if (!isToolName(name)) return undefined;

    const toolMap: Record<TelegramMCPToolName, TelegramMCPToolHandler> = {
      get_chats: tools.getChats,
      list_chats: tools.listChats,
      get_chat: tools.getChat,
      create_group: tools.createGroup,
      invite_to_group: tools.inviteToGroup,
      create_channel: tools.createChannel,
      edit_chat_title: tools.editChatTitle,
      delete_chat_photo: tools.deleteChatPhoto,
      leave_chat: tools.leaveChat,
      get_participants: tools.getParticipants,
      get_admins: tools.getAdmins,
      get_banned_users: tools.getBannedUsers,
      promote_admin: tools.promoteAdmin,
      demote_admin: tools.demoteAdmin,
      ban_user: tools.banUser,
      unban_user: tools.unbanUser,
      get_invite_link: tools.getInviteLink,
      export_chat_invite: tools.exportChatInvite,
      import_chat_invite: tools.importChatInvite,
      join_chat_by_link: tools.joinChatByLink,
      subscribe_public_channel: tools.subscribePublicChannel,
      get_messages: tools.getMessages,
      list_messages: tools.listMessages,
      list_topics: tools.listTopics,
      send_message: tools.sendMessage,
      reply_to_message: tools.replyToMessage,
      edit_message: tools.editMessage,
      delete_message: tools.deleteMessage,
      forward_message: tools.forwardMessage,
      pin_message: tools.pinMessage,
      unpin_message: tools.unpinMessage,
      mark_as_read: tools.markAsRead,
      get_message_context: tools.getMessageContext,
      get_history: tools.getHistory,
      get_pinned_messages: tools.getPinnedMessages,
      send_reaction: tools.sendReaction,
      remove_reaction: tools.removeReaction,
      get_message_reactions: tools.getMessageReactions,
      list_contacts: tools.listContacts,
      search_contacts: tools.searchContacts,
      add_contact: tools.addContact,
      delete_contact: tools.deleteContact,
      block_user: tools.blockUser,
      unblock_user: tools.unblockUser,
      get_blocked_users: tools.getBlockedUsers,
      get_me: tools.getMe,
      update_profile: tools.updateProfile,
      get_user_photos: tools.getUserPhotos,
      get_user_status: tools.getUserStatus,
      mute_chat: tools.muteChat,
      unmute_chat: tools.unmuteChat,
      archive_chat: tools.archiveChat,
      unarchive_chat: tools.unarchiveChat,
      get_privacy_settings: tools.getPrivacySettings,
      set_privacy_settings: tools.setPrivacySettings,
      list_inline_buttons: tools.listInlineButtons,
      press_inline_button: tools.pressInlineButton,
      save_draft: tools.saveDraft,
      get_drafts: tools.getDrafts,
      clear_draft: tools.clearDraft,
      search_public_chats: tools.searchPublicChats,
      search_messages: tools.searchMessages,
      resolve_username: tools.resolveUsername,
      get_media_info: tools.getMediaInfo,
      get_recent_actions: tools.getRecentActions,
      create_poll: tools.createPoll,
      get_bot_info: tools.getBotInfo,
      set_bot_commands: tools.setBotCommands,
      set_profile_photo: tools.setProfilePhoto,
      delete_profile_photo: tools.deleteProfilePhoto,
      edit_chat_photo: tools.editChatPhoto,
      get_sticker_sets: tools.getStickerSets,
      get_gif_search: tools.getGifSearch,
      get_contact_ids: tools.getContactIds,
      import_contacts: tools.importContacts,
      export_contacts: tools.exportContacts,
      get_direct_chat_by_contact: tools.getDirectChatByContact,
      get_contact_chats: tools.getContactChats,
      get_last_interaction: tools.getLastInteraction,
    };

    return toolMap[name];
  }
}
