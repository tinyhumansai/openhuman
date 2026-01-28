import type { MCPTool, MCPToolResult } from "../../types";
import type { TelegramMCPContext } from "../types";
import { ErrorCategory, logAndFormatError } from '../../errorHandler';
import { mtprotoService } from '../../../../services/mtprotoService';
import { Api } from 'telegram';

export const tool: MCPTool = {
  name: "delete_profile_photo",
  description: "Delete profile photo",
  inputSchema: { type: "object", properties: {} },
};

export async function deleteProfilePhoto(
  _args: Record<string, unknown>,
  _context: TelegramMCPContext,
): Promise<MCPToolResult> {
  try {
    const client = mtprotoService.getClient();

    const photos = await mtprotoService.withFloodWaitHandling(async () => {
      return client.invoke(
        new Api.photos.GetUserPhotos({
          userId: new Api.InputUserSelf(),
          offset: 0,
          maxId: BigInt(0),
          limit: 1,
        }),
      );
    });

    if (!photos || !('photos' in photos) || !Array.isArray(photos.photos) || photos.photos.length === 0) {
      return { content: [{ type: 'text', text: 'No profile photo to delete.' }] };
    }

    const photo = photos.photos[0] as any;

    await mtprotoService.withFloodWaitHandling(async () => {
      await client.invoke(
        new Api.photos.DeletePhotos({
          id: [new Api.InputPhoto({ id: photo.id, accessHash: photo.accessHash, fileReference: photo.fileReference })],
        }),
      );
    });

    return { content: [{ type: 'text', text: 'Profile photo deleted.' }] };
  } catch (error) {
    return logAndFormatError(
      'delete_profile_photo',
      error instanceof Error ? error : new Error(String(error)),
      ErrorCategory.PROFILE,
    );
  }
}
