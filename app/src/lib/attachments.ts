/**
 * Utilities for multimodal chat attachments.
 *
 * Images are embedded in the message text as `[IMAGE:<data-uri>]` markers,
 * which the Rust agent harness (`agent/multimodal.rs`) parses and forwards to
 * the inference provider. The backend supports up to 4 images per message at
 * 8 MB each by default (governed by `MultimodalConfig`).
 */

export const ATTACHMENT_MAX_IMAGES = 4;
export const ATTACHMENT_MAX_SIZE_BYTES = 8 * 1024 * 1024; // 8 MB

export const ALLOWED_IMAGE_MIME_TYPES = [
  'image/png',
  'image/jpeg',
  'image/webp',
  'image/gif',
  'image/bmp',
] as const;

export type AllowedImageMimeType = (typeof ALLOWED_IMAGE_MIME_TYPES)[number];

export interface Attachment {
  id: string;
  file: File;
  dataUri: string;
  mimeType: AllowedImageMimeType;
}

export type AttachmentError =
  | { code: 'unsupported_type'; mimeType: string }
  | { code: 'too_large'; sizeBytes: number; maxBytes: number }
  | { code: 'too_many'; max: number }
  | { code: 'read_failed'; reason: string };

export function isAllowedMimeType(mime: string): mime is AllowedImageMimeType {
  return (ALLOWED_IMAGE_MIME_TYPES as readonly string[]).includes(mime);
}

export function fileToDataUri(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as string);
    reader.onerror = () => reject(new Error(`Failed to read file: ${file.name}`));
    reader.readAsDataURL(file);
  });
}

export async function validateAndReadFile(
  file: File,
  existingCount: number
): Promise<{ attachment: Attachment } | { error: AttachmentError }> {
  if (existingCount >= ATTACHMENT_MAX_IMAGES) {
    return { error: { code: 'too_many', max: ATTACHMENT_MAX_IMAGES } };
  }

  if (!isAllowedMimeType(file.type)) {
    return { error: { code: 'unsupported_type', mimeType: file.type || 'unknown' } };
  }

  if (file.size > ATTACHMENT_MAX_SIZE_BYTES) {
    return {
      error: { code: 'too_large', sizeBytes: file.size, maxBytes: ATTACHMENT_MAX_SIZE_BYTES },
    };
  }

  try {
    const dataUri = await fileToDataUri(file);
    return {
      attachment: {
        id: globalThis.crypto.randomUUID(),
        file,
        dataUri,
        mimeType: file.type as AllowedImageMimeType,
      },
    };
  } catch (err) {
    return {
      error: { code: 'read_failed', reason: err instanceof Error ? err.message : String(err) },
    };
  }
}

/**
 * Compose the final message string by appending `[IMAGE:<data-uri>]` markers
 * for each attachment after the user's text. The Rust backend parses these
 * markers in `parse_image_markers` and strips them from the visible message
 * before forwarding clean text + image payload to the inference provider.
 */
export function buildMessageWithAttachments(text: string, attachments: Attachment[]): string {
  if (attachments.length === 0) return text;
  const markers = attachments.map(a => `[IMAGE:${a.dataUri}]`).join(' ');
  return text.trim() ? `${text.trim()} ${markers}` : markers;
}

/**
 * Parse `[IMAGE:<data-uri>]` markers out of a stored message string.
 * Returns the clean text (markers removed) and the list of data URIs found.
 */
export function parseMessageImages(content: string): { text: string; dataUris: string[] } {
  const dataUris: string[] = [];
  const text = content
    .replace(/\[IMAGE:([^\]]+)\]/g, (_match, uri: string) => {
      dataUris.push(uri);
      return '';
    })
    .trim();
  return { text, dataUris };
}

export function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
