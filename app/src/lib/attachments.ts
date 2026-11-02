/**
 * Utilities for multimodal chat attachments.
 *
 * Originals travel as typed `ContentPart`s ({@link attachmentsToParts}) next to
 * the message text; the core stages them and returns durable opaque typed
 * references. {@link parseAttachmentReferences} remains for legacy marker
 * records. The `[IMAGE:]`/`[FILE:]` marker grammar is an internal staging
 * format of the core and is no longer built here.
 */
import debugFactory from 'debug';

import type { ContentPart } from '../services/chatWire';

const debug = debugFactory('chat:attachments');

const ALLOWED_IMAGE_MIME_TYPES = [
  'image/png',
  'image/jpeg',
  'image/webp',
  'image/gif',
  'image/bmp',
] as const;

export type AllowedImageMimeType = (typeof ALLOWED_IMAGE_MIME_TYPES)[number];

const ALLOWED_FILE_MIME_TYPES = [
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

// Known video types select a video chip; every other MIME is still accepted
// as an original file. Capability routing and extraction belong to the core.
const ALLOWED_VIDEO_MIME_TYPES = ['video/mp4', 'video/quicktime', 'video/webm'] as const;
export type AllowedVideoMimeType = (typeof ALLOWED_VIDEO_MIME_TYPES)[number];
export type AllowedAttachmentMimeType = string;
export type AttachmentKind = 'image' | 'file' | 'video';
export const ALLOWED_ATTACHMENT_MIME_TYPES = [
  ...ALLOWED_IMAGE_MIME_TYPES,
  ...ALLOWED_FILE_MIME_TYPES,
  ...ALLOWED_VIDEO_MIME_TYPES,
] as const;

// Original image uploads and all other original files have separate budgets.
export const ATTACHMENT_MAX_IMAGES = 4;
export const ATTACHMENT_MAX_FILES = 4;
export const ATTACHMENT_MAX_IMAGE_SIZE_BYTES = 8 * 1024 * 1024; // 8 MB
export const ATTACHMENT_MAX_FILE_SIZE_BYTES = 16 * 1024 * 1024; // 16 MB
export const ATTACHMENT_MAX_VIDEO_SIZE_BYTES = ATTACHMENT_MAX_FILE_SIZE_BYTES;
// Default number of stills for callers explicitly requesting a video preview.
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
  // Optional preview frames only; originals are always sent as FILE markers.
  frames?: string[];
}

type AttachmentError =
  | { code: 'too_large'; sizeBytes: number; maxBytes: number }
  | { code: 'too_many'; kind: AttachmentKind; max: number }
  | { code: 'read_failed'; reason: string };

export function isAllowedMimeType(mime: string): mime is AllowedImageMimeType {
  return (ALLOWED_IMAGE_MIME_TYPES as readonly string[]).includes(mime);
}

export function isVideoMimeType(mime: string): mime is AllowedVideoMimeType {
  return (ALLOWED_VIDEO_MIME_TYPES as readonly string[]).includes(mime);
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
  // Blob normalizes MIME text to lowercase, including arbitrary parameters.
  // Insert the encoded filename after reading so its case stays intact.
  const typedBlob = new Blob([blob], { type: mimeType.split(';')[0] });
  const uri = await fileToDataUri(typedBlob);
  return `data:${mimeType};base64,${uri.slice(uri.indexOf(',') + 1)}`;
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

  const dataUri = await blobToDataUri(file, `${mimeType};name=${encodeDataUriParam(file.name)}`);
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
 * This optional preview helper does not affect original upload acceptance.
 * Throws if the browser cannot decode the file. Requires a codec-capable browser;
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

/** Number of image markers sent for an original attachment. */
export function imageMarkerCost(kind: AttachmentKind): number {
  return kind === 'image' ? 1 : 0;
}

export async function validateAndReadFile(
  file: File,
  existingImageMarkers: number,
  existingFileCount = 0,
  // Kept for compatibility with callers: capabilities affect core routing,
  // never whether an original may be uploaded.
  _allowImages = true
): Promise<{ attachment: Attachment } | { error: AttachmentError }> {
  const mimeType = file.type || 'application/octet-stream';
  const kind = attachmentKindForMime(mimeType);
  if (kind !== 'image') {
    if (existingFileCount >= ATTACHMENT_MAX_FILES) {
      return { error: { code: 'too_many', kind: 'file', max: ATTACHMENT_MAX_FILES } };
    }
  } else if (existingImageMarkers >= ATTACHMENT_MAX_IMAGES) {
    return { error: { code: 'too_many', kind: 'image', max: ATTACHMENT_MAX_IMAGES } };
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
    const { dataUri, payloadSizeBytes, compressed } = await buildAttachmentDataUri(file, mimeType);
    const previewUri = kind === 'image' ? await fileToDataUri(file) : undefined;
    return {
      attachment: {
        id: globalThis.crypto.randomUUID(),
        kind,
        file,
        dataUri,
        previewUri,
        mimeType,
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
 * The typed wire form of composer attachments (`ContentPart[]`): an `image`,
 * `video` or `file` part whose media carries the original as a `data:` URI
 * plus its name, type and size. Sent next to the message text through the
 * `attachments` param of `channel.web_chat`, `threads.message_append` and
 * `threads.edit_message`; the core stages the originals. The text is never
 * touched, so no `[IMAGE:]`/`[FILE:]` marker is built here.
 */
export function attachmentsToParts(attachments: Attachment[]): ContentPart[] {
  return attachments.map((a): ContentPart => {
    const media = {
      url: a.dataUri,
      contentType: a.mimeType,
      fileName: a.file.name,
      sizeBytes: a.originalSizeBytes,
      codec: null,
      durationMs: null,
      height: null,
      localCachePath: null,
      path: null,
      platformMessageId: null,
      transcribed: false,
      width: null,
    };
    if (a.kind === 'image') {
      return { kind: 'image', altText: null, caption: null, media: { ...media, kind: 'image' } };
    }
    if (a.kind === 'video') {
      return {
        kind: 'video',
        caption: null,
        durationMs: null,
        thumbnail: null,
        media: { ...media, kind: 'video' },
      };
    }
    return {
      kind: 'file',
      fileName: a.file.name,
      caption: null,
      media: { ...media, kind: 'document' },
    };
  });
}

/** Workspace references returned by the core after it has saved originals. */
export interface AttachmentReference {
  path: string;
  name: string;
  mime: string;
  size_bytes: number;
}

/** Decode durable references without exposing raw JSON in message bubbles. */
export function parseAttachmentReferences(content: string): {
  text: string;
  attachments: AttachmentReference[];
} {
  const attachments: AttachmentReference[] = [];
  const text = content
    .replace(/\[ATTACHMENT:([^\]]+)\]/g, (marker, encoded: string) => {
      try {
        const decoded = new URLSearchParams(`value=${encoded}`).get('value');
        const value: unknown = JSON.parse(decoded ?? '');
        if (!value || typeof value !== 'object') return marker;
        const file = value as Partial<AttachmentReference>;
        if (
          typeof file.path !== 'string' ||
          typeof file.name !== 'string' ||
          typeof file.mime !== 'string' ||
          typeof file.size_bytes !== 'number' ||
          !Number.isSafeInteger(file.size_bytes) ||
          file.size_bytes < 0
        )
          return marker;
        // These are workspace references, never URLs or absolute browser targets.
        const components = file.path.split('/');
        if (
          !file.path ||
          file.path.includes('\\') ||
          file.path.includes(':') ||
          file.path.includes('\0') ||
          components.some(part => part === '' || part === '.' || part === '..')
        )
          return marker;
        attachments.push(file as AttachmentReference);
        return '';
      } catch {
        return marker;
      }
    })
    .replace(/ {2,}/g, ' ')
    .trim();
  return { text, attachments };
}

/** Display metadata excludes payload bytes; originals travel only in upload markers. */
export function attachmentMetadata(attachments: Attachment[]): Record<string, unknown> {
  return attachments.length === 0
    ? {}
    : {
        attachmentCount: attachments.length,
        attachmentNames: attachments.map(file => file.file.name),
        attachmentKinds: attachments.map(file => file.kind),
        attachmentCompressed: attachments.map(file => file.compressed),
      };
}

/**
 * Parse `[IMAGE:<data-uri>]` and `[FILE:<data-uri>]` markers out of a stored message string.
 * Returns the clean text (markers removed) and the list of image data URIs found.
 * File markers are stripped from text but not returned (file data lives in extraMetadata).
 */
export function parseMessageImages(content: string): { text: string; dataUris: string[] } {
  const dataUris: string[] = [];
  const text = parseAttachmentReferences(content)
    .text.replace(/\[IMAGE:([^\]]+)\]/g, (_match, uri: string) => {
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
