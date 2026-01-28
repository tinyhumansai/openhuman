import type { MCPTool, MCPToolResult } from "../../types";
import type { TelegramMCPContext } from "../types";
import { ErrorCategory, logAndFormatError } from '../../errorHandler';
import { validateId } from '../../validation';
import { getChatById } from '../telegramApi';
import { mtprotoService } from '../../../../services/mtprotoService';
import { Api } from 'telegram';

export const tool: MCPTool = {
  name: "create_poll",
  description: "Create a poll in a chat",
  inputSchema: {
    type: "object",
    properties: {
      chat_id: { type: "string", description: "Chat ID or username" },
      question: { type: "string", description: "Poll question" },
      options: { type: "array", description: "Poll options" },
    },
    required: ["chat_id", "question", "options"],
  },
};

export async function createPoll(
  args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const chatId = validateId(args.chat_id, 'chat_id');
    const question = typeof args.question === 'string' ? args.question : '';
    const options = Array.isArray(args.options) ? args.options.map(String) : [];

    if (!question) return { content: [{ type: 'text', text: 'question is required' }], isError: true };
    if (options.length < 2) return { content: [{ type: 'text', text: 'At least 2 options are required' }], isError: true };

    const chat = getChatById(chatId);
    if (!chat) return { content: [{ type: 'text', text: 'Chat not found: ' + chatId }], isError: true };

    const client = mtprotoService.getClient();
    const entity = chat.username ? chat.username : chat.id;

    await mtprotoService.withFloodWaitHandling(async () => {
      const inputPeer = await client.getInputEntity(entity);
      await client.invoke(
        new Api.messages.SendMedia({
          peer: inputPeer,
          media: new Api.InputMediaPoll({
            poll: new Api.Poll({
              id: BigInt(0),
              question: new Api.TextWithEntities({ text: question, entities: [] }),
              answers: options.map((opt, i) =>
                new Api.PollAnswer({
                  text: new Api.TextWithEntities({ text: opt, entities: [] }),
                  option: Buffer.from([i]),
                }),
              ),
            }),
          }),
          message: '',
          randomId: BigInt(Math.floor(Math.random() * Number.MAX_SAFE_INTEGER)),
        }),
      );
    });

    return { content: [{ type: 'text', text: 'Poll created: ' + question }] };
  } catch (error) {
    return logAndFormatError(
      'create_poll',
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.MSG,
    );
  }
}
