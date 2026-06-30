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

// Video formats accepted by the composer. The original video is never sent to
// the provider — instead we sample a few still frames client-side and forward
// them through the existing `[IMAGE:]` vision path (see `extractVideoFrames` +
// `buildMessageWithAttachments`). So a vision-capable tier is required, the same
// gate as images; audio and motion between frames are not conveyed.
export const ALLOWED_VIDEO_MIME_TYPES = ['video/mp4', 'video/quicktime', 'video/webm'] as const;

export type AllowedVideoMimeType = (typeof ALLOWED_VIDEO_MIME_TYPES)[number];
export type AllowedAttachmentMimeType =
  | AllowedImageMimeType
  | AllowedFileMimeType
  | AllowedVideoMimeType;
export type AttachmentKind = 'image' | 'file' | 'video';

export const ALLOWED_ATTACHMENT_MIME_TYPES = [
  ...ALLOWED_IMAGE_MIME_TYPES,
  ...ALLOWED_FILE_MIME_TYPES,
  ...ALLOWED_VIDEO_MIME_TYPES,
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

// Shared image-marker budget per message. Images cost 1 marker each; a video
// costs VIDEO_FRAME_COUNT markers (its sampled frames). Mirrors the core default
// `multimodal.max_images` (src/openhuman/config/schema/tools/multimodal.rs) — the
// core counts every `[IMAGE:]` marker (frames included) and errors on overflow,
// so the composer must budget images + video frames against this single cap.
export const ATTACHMENT_MAX_IMAGES = 4;
export const ATTACHMENT_MAX_FILES = 4;
export const ATTACHMENT_MAX_IMAGE_SIZE_BYTES = 8 * 1024 * 1024; // 8 MB
export const ATTACHMENT_MAX_FILE_SIZE_BYTES = 16 * 1024 * 1024; // 16 MB
export const ATTACHMENT_MAX_VIDEO_SIZE_BYTES = 50 * 1024 * 1024; // 50 MB
export const ATTACHMENT_MAX_SIZE_BYTES = ATTACHMENT_MAX_IMAGE_SIZE_BYTES;
// Still frames sampled from a video and forwarded as `[IMAGE:]` markers. 2 keeps
// a clip within the 4-marker budget alongside other attachments (e.g. 1 video +
// 2 images, or 2 videos).
export const VIDEO_FRAME_COUNT = 2;

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
  // Only set for `kind: 'video'`: the still frames sampled from the clip,
  // expanded into `[IMAGE:]` markers at send time. The chip itself shows a
  // single poster (the first frame) via `previewUri`.
  frames?: string[];
}

export type AttachmentError =
  | { code: 'unsupported_type'; mimeType: string }
  | { code: 'too_large'; sizeBytes: number; maxBytes: number }
  | { code: 'too_many'; kind: AttachmentKind; max: number }
  | { code: 'image_not_supported' }
  | { code: 'video_not_supported' }
  | { code: 'read_failed'; reason: string };

export function isAllowedMimeType(mime: string): mime is AllowedImageMimeType {
  return (ALLOWED_IMAGE_MIME_TYPES as readonly string[]).includes(mime);
}

export function isVideoMimeType(mime: string): mime is AllowedVideoMimeType {
  return (ALLOWED_VIDEO_MIME_TYPES as readonly string[]).includes(mime);
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
  | AllowedVideoMimeType
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
    (ALLOWED_VIDEO_MIME_TYPES as readonly string[]).includes(mime) ||
    (EXTRACTABLE_FILE_MIME_TYPES as readonly string[]).includes(mime)
  );
}

export function attachmentKindForMime(mime: AllowedAttachmentMimeType): AttachmentKind {
  if (isAllowedMimeType(mime)) return 'image';
  if (isVideoMimeType(mime)) return 'video';
  return 'file';
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

/** Evenly spread `count` sample points across the 0.1–0.9 span of the clip. */
function sampleFractions(count: number): number[] {
  if (count <= 1) return [0.1];
  const fractions: number[] = [];
  for (let i = 0; i < count; i++) {
    fractions.push(0.1 + (0.8 * i) / (count - 1));
  }
  return fractions;
}

function seekVideo(video: HTMLVideoElement, time: number): Promise<void> {
  return new Promise((resolve, reject) => {
    // Assigning currentTime to (approximately) its current value is a no-op and
    // never fires `seeked` (HTML spec) — which would hang for zero-length clips
    // or a first frame already at t=0. Short-circuit those.
    if (Math.abs(video.currentTime - time) < 0.01) {
      resolve();
      return;
    }
    const cleanup = () => {
      video.removeEventListener('seeked', onSeeked);
      video.removeEventListener('error', onError);
    };
    const onSeeked = () => {
      cleanup();
      resolve();
    };
    const onError = () => {
      cleanup();
      reject(new Error('video seek failed'));
    };
    video.addEventListener('seeked', onSeeked);
    video.addEventListener('error', onError);
    video.currentTime = time;
  });
}

/**
 * Sample `count` still frames from a video file as JPEG data URIs by decoding it
 * in a detached `<video>` element and painting each seek point onto a `<canvas>`.
 * The full clip is never uploaded — only these frames ride the `[IMAGE:]` vision
 * path. Throws if the browser can't decode the file (the caller maps that to a
 * `read_failed` error). Requires a real codec-capable runtime (CEF/Chromium);
 * jsdom can't decode video, so unit tests stub {@link videoFrameExtractor}.
 */
async function extractVideoFramesImpl(
  file: File,
  count: number = VIDEO_FRAME_COUNT
): Promise<string[]> {
  const url = URL.createObjectURL(file);
  const video = document.createElement('video');
  video.muted = true;
  video.preload = 'auto';
  video.src = url;
  try {
    await new Promise<void>((resolve, reject) => {
      video.onloadedmetadata = () => resolve();
      video.onerror = () => reject(new Error('video metadata load failed'));
    });
    const duration = Number.isFinite(video.duration) && video.duration > 0 ? video.duration : 0;
    const canvas = document.createElement('canvas');
    canvas.width = video.videoWidth || 320;
    canvas.height = video.videoHeight || 240;
    const ctx = canvas.getContext('2d');
    if (!ctx) throw new Error('canvas 2d context unavailable');

    const frames: string[] = [];
    for (const fraction of sampleFractions(count)) {
      const target = duration ? Math.min(duration * fraction, Math.max(duration - 0.05, 0)) : 0;
      await seekVideo(video, target);
      ctx.drawImage(video, 0, 0, canvas.width, canvas.height);
      frames.push(canvas.toDataURL('image/jpeg', 0.7));
    }
    debug('[chat:attachments] video_frames name=%s count=%d', file.name, frames.length);
    return frames;
  } finally {
    URL.revokeObjectURL(url);
    video.removeAttribute('src');
  }
}

/**
 * Indirection seam so unit tests can stub frame extraction (jsdom has no video
 * decoder). Production calls `extract` directly.
 */
export const videoFrameExtractor = { extract: extractVideoFramesImpl };

export function extractVideoFrames(file: File, count?: number): Promise<string[]> {
  return videoFrameExtractor.extract(file, count);
}

/** Image-marker cost of an attachment kind (video = its sampled frames). */
export function imageMarkerCost(kind: AttachmentKind): number {
  if (kind === 'image') return 1;
  if (kind === 'video') return VIDEO_FRAME_COUNT;
  return 0;
}

export async function validateAndReadFile(
  file: File,
  // Image-marker slots already consumed this message: 1 per image + VIDEO_FRAME_COUNT
  // per video. Images and videos share one budget (ATTACHMENT_MAX_IMAGES) because
  // the core counts every `[IMAGE:]` marker — frames included — and rejects overflow.
  existingImageMarkers: number,
  existingFileCount = 0,
  // When `false` (the active chat model isn't vision-capable), image AND video
  // files are rejected (video is conveyed as sampled frames through the vision
  // path); documents (PDF/Word/etc.) still flow. Defaults `true` so non-chat
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
  if (!allowImages && kind === 'video') {
    return { error: { code: 'video_not_supported' } };
  }

  if (kind === 'file') {
    if (existingFileCount >= ATTACHMENT_MAX_FILES) {
      return { error: { code: 'too_many', kind: 'file', max: ATTACHMENT_MAX_FILES } };
    }
  } else {
    // image or video: budget against the shared image-marker cap.
    if (existingImageMarkers + imageMarkerCost(kind) > ATTACHMENT_MAX_IMAGES) {
      return { error: { code: 'too_many', kind: 'image', max: ATTACHMENT_MAX_IMAGES } };
    }
  }

  const maxBytes =
    kind === 'image'
      ? ATTACHMENT_MAX_IMAGE_SIZE_BYTES
      : kind === 'video'
        ? ATTACHMENT_MAX_VIDEO_SIZE_BYTES
        : ATTACHMENT_MAX_FILE_SIZE_BYTES;
  if (file.size > maxBytes) {
    return { error: { code: 'too_large', sizeBytes: file.size, maxBytes } };
  }

  try {
    if (kind === 'video') {
      const frames = await videoFrameExtractor.extract(file);
      if (frames.length === 0) {
        return { error: { code: 'read_failed', reason: 'no frames extracted' } };
      }
      return {
        attachment: {
          id: globalThis.crypto.randomUUID(),
          kind: 'video',
          file,
          // The clip is represented to the agent by its frames, not a data URI;
          // the poster (first frame) drives the chip thumbnail.
          dataUri: frames[0],
          previewUri: frames[0],
          mimeType: file.type,
          originalSizeBytes: file.size,
          payloadSizeBytes: file.size,
          compressed: false,
          frames,
        },
      };
    }

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
    .map(a => {
      if (a.kind === 'image') return `[IMAGE:${a.dataUri}]`;
      if (a.kind === 'file') return `[FILE:${a.dataUri}]`;
      // Video: forward each sampled still as its own image marker so the agent
      // "sees" the clip through the existing vision path.
      return (a.frames ?? []).map(frame => `[IMAGE:${frame}]`).join(' ');
    })
    .filter(marker => marker.length > 0)
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
