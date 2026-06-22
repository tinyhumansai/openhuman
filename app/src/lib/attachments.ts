/**
 * Utilities for multimodal chat attachments.
 *
 * Images are embedded as `[IMAGE:<data-uri>]` markers. Other supported files
 * are embedded as `[FILE:<data-uri>]` markers. The Rust agent harness
 * (`agent/multimodal.rs`) parses, validates, and expands both shapes before
 * the provider call.
 */
import debugFactory from 'debug';

const debug = debugFactory('chat:attachments');

export const ALLOWED_IMAGE_MIME_TYPES = [
  'image/png',
  'image/jpeg',
  'image/webp',
  'image/gif',
  'image/bmp',
] as const;

export type AllowedImageMimeType = (typeof ALLOWED_IMAGE_MIME_TYPES)[number];

export const ALLOWED_FILE_MIME_TYPES = [
  'application/pdf',
  'text/plain',
  'text/csv',
  'text/markdown',
  'application/zip',
  'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
  'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
  'application/vnd.openxmlformats-officedocument.presentationml.presentation',
  'application/octet-stream',
] as const;

export type AllowedFileMimeType = (typeof ALLOWED_FILE_MIME_TYPES)[number];
export type AllowedAttachmentMimeType = AllowedImageMimeType | AllowedFileMimeType;
export type AttachmentKind = 'image' | 'file';

export const ALLOWED_ATTACHMENT_MIME_TYPES = [
  ...ALLOWED_IMAGE_MIME_TYPES,
  ...ALLOWED_FILE_MIME_TYPES,
] as const;

// Document formats the backend actually text-extracts (PDF via pdf_extract;
// TXT/Markdown via UTF-8). DOCX/PPTX/XLSX/ZIP are intentionally excluded — the
// agent would only see a reference stub, not their content. `text/csv` is also
// deliberately left out: the backend *can* extract it, but the chat composer is
// scoped to PDF/TXT/Markdown by product decision (revisit here if CSV is wanted).
// Used by the ingest validator below, not by a native `accept` filter:
// Chromium/CEF on macOS greys valid files at the open panel regardless of the
// filter shape, so selection is gated in `validateAndReadFile` after the user
// picks, not at the dialog.
const EXTRACTABLE_FILE_MIME_TYPES = ['application/pdf', 'text/plain', 'text/markdown'] as const;

export const ATTACHMENT_MAX_IMAGES = 4;
export const ATTACHMENT_MAX_FILES = 4;
export const ATTACHMENT_MAX_IMAGE_SIZE_BYTES = 8 * 1024 * 1024; // 8 MB
export const ATTACHMENT_MAX_FILE_SIZE_BYTES = 16 * 1024 * 1024; // 16 MB
export const ATTACHMENT_MAX_SIZE_BYTES = ATTACHMENT_MAX_IMAGE_SIZE_BYTES;

export interface Attachment {
  id: string;
  kind: AttachmentKind;
  file: File;
  dataUri: string;
  previewUri?: string;
  mimeType: AllowedAttachmentMimeType;
  originalSizeBytes: number;
  payloadSizeBytes: number;
  compressed: boolean;
}

export type AttachmentError =
  | { code: 'unsupported_type'; mimeType: string }
  | { code: 'too_large'; sizeBytes: number; maxBytes: number }
  | { code: 'too_many'; kind: AttachmentKind; max: number }
  | { code: 'image_not_supported' }
  | { code: 'read_failed'; reason: string };

export function isAllowedMimeType(mime: string): mime is AllowedImageMimeType {
  return (ALLOWED_IMAGE_MIME_TYPES as readonly string[]).includes(mime);
}

export function isAllowedAttachmentMimeType(mime: string): mime is AllowedAttachmentMimeType {
  return (ALLOWED_ATTACHMENT_MIME_TYPES as readonly string[]).includes(mime);
}

/**
 * The exact MIME set the ingest validator accepts — images plus the
 * text-extractable documents. A strict subset of {@link AllowedAttachmentMimeType}
 * (which also lists reference-only types like CSV/DOCX/ZIP that we reject).
 */
export type SupportedAttachmentMimeType =
  | AllowedImageMimeType
  | (typeof EXTRACTABLE_FILE_MIME_TYPES)[number];

/**
 * Stricter gate than {@link isAllowedAttachmentMimeType}: only the formats the
 * backend actually reads — images, plus the text-extractable documents (PDF via
 * pdf_extract; TXT/Markdown via UTF-8). DOCX/PPTX/XLSX/ZIP are excluded so they
 * can't be attached as content-less reference stubs. Applied on every ingest
 * path (picker, drag-drop, paste).
 */
export function isSupportedAttachmentMimeType(mime: string): mime is SupportedAttachmentMimeType {
  return (
    (ALLOWED_IMAGE_MIME_TYPES as readonly string[]).includes(mime) ||
    (EXTRACTABLE_FILE_MIME_TYPES as readonly string[]).includes(mime)
  );
}

export function attachmentKindForMime(mime: AllowedAttachmentMimeType): AttachmentKind {
  return isAllowedMimeType(mime) ? 'image' : 'file';
}

export function fileToDataUri(file: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    const name = file instanceof File ? file.name : 'blob';
    reader.onload = () => resolve(reader.result as string);
    reader.onerror = () => reject(new Error(`Failed to read file: ${name}`));
    reader.readAsDataURL(file);
  });
}

async function blobToDataUri(blob: Blob, mimeType: string): Promise<string> {
  const namedBlob = new Blob([blob], { type: mimeType });
  return fileToDataUri(namedBlob);
}

async function gzipBlob(file: File): Promise<Blob | null> {
  if (!('CompressionStream' in globalThis)) return null;

  try {
    const compressionStream = new CompressionStream('gzip');
    const compressed = file.stream().pipeThrough(compressionStream);
    return await new Response(compressed).blob();
  } catch (error) {
    debug('[chat:attachments] gzip_failed name=%s error=%o', file.name, error);
    return null;
  }
}

function encodeDataUriParam(value: string): string {
  return encodeURIComponent(value).replace(/'/g, '%27');
}

async function buildAttachmentDataUri(
  file: File,
  mimeType: AllowedAttachmentMimeType
): Promise<{ dataUri: string; payloadSizeBytes: number; compressed: boolean }> {
  debug(
    '[chat:attachments] compression:start name=%s mime=%s size=%d',
    file.name,
    mimeType,
    file.size
  );

  const compressed = await gzipBlob(file);
  if (compressed && compressed.size < file.size) {
    const dataUri = await blobToDataUri(
      compressed,
      `application/gzip;original_mime=${encodeDataUriParam(mimeType)};name=${encodeDataUriParam(file.name)}`
    );
    debug(
      '[chat:attachments] compression:ok name=%s original=%d compressed=%d',
      file.name,
      file.size,
      compressed.size
    );
    return { dataUri, payloadSizeBytes: compressed.size, compressed: true };
  }

  const dataUri = await fileToDataUri(file);
  debug(
    '[chat:attachments] compression:skipped name=%s original=%d compressed=%s',
    file.name,
    file.size,
    compressed?.size ?? 'unavailable'
  );
  return { dataUri, payloadSizeBytes: file.size, compressed: false };
}

export async function validateAndReadFile(
  file: File,
  existingCount: number,
  existingFileCount = 0,
  // When `false` (the active chat model isn't vision-capable), image files are
  // rejected; documents (PDF/Word/etc.) still flow. Defaults `true` so non-chat
  // callers are unaffected.
  allowImages = true
): Promise<{ attachment: Attachment } | { error: AttachmentError }> {
  if (!isSupportedAttachmentMimeType(file.type)) {
    return { error: { code: 'unsupported_type', mimeType: file.type || 'unknown' } };
  }

  const kind = attachmentKindForMime(file.type);
  if (!allowImages && kind === 'image') {
    return { error: { code: 'image_not_supported' } };
  }
  const maxCount = kind === 'image' ? ATTACHMENT_MAX_IMAGES : ATTACHMENT_MAX_FILES;
  const count = kind === 'image' ? existingCount : existingFileCount;
  if (count >= maxCount) {
    return { error: { code: 'too_many', kind, max: maxCount } };
  }

  const maxBytes =
    kind === 'image' ? ATTACHMENT_MAX_IMAGE_SIZE_BYTES : ATTACHMENT_MAX_FILE_SIZE_BYTES;
  if (file.size > maxBytes) {
    return { error: { code: 'too_large', sizeBytes: file.size, maxBytes } };
  }

  try {
    const { dataUri, payloadSizeBytes, compressed } = await buildAttachmentDataUri(file, file.type);
    const previewUri = kind === 'image' ? await fileToDataUri(file) : undefined;
    return {
      attachment: {
        id: globalThis.crypto.randomUUID(),
        kind,
        file,
        dataUri,
        previewUri,
        mimeType: file.type,
        originalSizeBytes: file.size,
        payloadSizeBytes,
        compressed,
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
 * for image attachments and `[FILE:<data-uri>]` markers for other supported
 * files after the user's text. The Rust agent harness parses and strips these
 * markers before forwarding clean text and attachment payloads to the provider.
 */
export function buildMessageWithAttachments(text: string, attachments: Attachment[]): string {
  if (attachments.length === 0) return text;
  const markers = attachments
    .map(a => (a.kind === 'image' ? `[IMAGE:${a.dataUri}]` : `[FILE:${a.dataUri}]`))
    .join(' ');
  return text.trim() ? `${text.trim()} ${markers}` : markers;
}

/**
 * Parse `[IMAGE:<data-uri>]` and `[FILE:<data-uri>]` markers out of a stored message string.
 * Returns the clean text (markers removed) and the list of image data URIs found.
 * File markers are stripped from text but not returned (file data lives in extraMetadata).
 */
export function parseMessageImages(content: string): { text: string; dataUris: string[] } {
  const dataUris: string[] = [];
  const text = content
    .replace(/\[IMAGE:([^\]]+)\]/g, (_match, uri: string) => {
      dataUris.push(uri);
      return '';
    })
    .replace(/\[FILE:([^\]]+)\]/g, '') // Strip file markers
    // Collapse only runs of plain spaces (not \s) left behind by marker
    // removal — using \s here would also eat intentional newlines/paragraph
    // breaks in the user's own text.
    .replace(/ {2,}/g, ' ')
    .trim();
  return { text, dataUris };
}

export function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
