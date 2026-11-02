import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  ALLOWED_ATTACHMENT_MIME_TYPES,
  type Attachment,
  ATTACHMENT_MAX_FILE_SIZE_BYTES,
  ATTACHMENT_MAX_IMAGE_SIZE_BYTES,
  ATTACHMENT_MAX_IMAGES,
  ATTACHMENT_MAX_VIDEO_SIZE_BYTES,
  attachmentKindForMime,
  attachmentsToParts,
  extractVideoFrames,
  fileToDataUri,
  formatFileSize,
  imageMarkerCost,
  isAllowedMimeType,
  isVideoMimeType,
  parseMessageImages,
  validateAndReadFile,
  videoFrameExtractor,
} from './attachments';

function makeFile(name: string, type: string, size = 1024): File {
  const blob = new Blob([new Uint8Array(size)], { type });
  return new File([blob], name, { type });
}

function makeAttachment(overrides: Partial<Attachment> = {}): Attachment {
  return {
    id: 'test-id',
    kind: 'image',
    file: makeFile('test.png', 'image/png'),
    dataUri: 'data:image/png;base64,abc',
    mimeType: 'image/png',
    originalSizeBytes: 1024,
    payloadSizeBytes: 1024,
    compressed: false,
    ...overrides,
  };
}

describe('isAllowedMimeType', () => {
  it('allows supported image types', () => {
    expect(isAllowedMimeType('image/png')).toBe(true);
    expect(isAllowedMimeType('image/jpeg')).toBe(true);
    expect(isAllowedMimeType('image/webp')).toBe(true);
    expect(isAllowedMimeType('image/gif')).toBe(true);
    expect(isAllowedMimeType('image/bmp')).toBe(true);
  });

  it('rejects unsupported types', () => {
    expect(isAllowedMimeType('application/pdf')).toBe(false);
    expect(isAllowedMimeType('text/plain')).toBe(false);
    expect(isAllowedMimeType('image/svg+xml')).toBe(false);
    expect(isAllowedMimeType('')).toBe(false);
  });
});

describe('allowed attachment MIME types', () => {
  it('includes images and supported document files', () => {
    expect(ALLOWED_ATTACHMENT_MIME_TYPES).toContain('image/png');
    expect(ALLOWED_ATTACHMENT_MIME_TYPES).toContain('application/pdf');
    expect(ALLOWED_ATTACHMENT_MIME_TYPES).toContain('text/plain');
  });
});

describe('fileToDataUri', () => {
  it('reads a file as a data URI', async () => {
    const file = makeFile('photo.png', 'image/png', 4);
    const uri = await fileToDataUri(file);
    expect(uri).toMatch(/^data:image\/png;base64,/);
  });
});

describe('validateAndReadFile', () => {
  it('preserves unknown MIME, filename and original bytes in the upload marker', async () => {
    const file = new File([Uint8Array.of(0, 255, 128)], 'archive ] résumé.bin');
    const result = await validateAndReadFile(file, 0);
    expect('attachment' in result).toBe(true);
    if ('attachment' in result) {
      expect(result.attachment.mimeType).toBe('application/octet-stream');
      expect(result.attachment.dataUri).toBe(
        'data:application/octet-stream;name=archive%20%5D%20r%C3%A9sum%C3%A9.bin;base64,AP+A'
      );
    }
  });

  it('uses reversible gzip transport while preserving original filename and MIME', async () => {
    const original = new TextEncoder().encode('a'.repeat(4096));
    const file = new File([original], 'Report.TXT', { type: 'text/plain' });
    Object.defineProperty(file, 'stream', {
      value: () =>
        new ReadableStream({
          start(controller) {
            controller.enqueue(original);
            controller.close();
          },
        }),
    });
    try {
      const result = await validateAndReadFile(file, 0);
      expect('attachment' in result).toBe(true);
      if ('attachment' in result) {
        expect(result.attachment.compressed).toBe(true);
        expect(result.attachment.dataUri).toMatch(
          /^data:application\/gzip;original_mime=text%2Fplain;name=Report.TXT;base64,/
        );
        const encoded = result.attachment.dataUri.split(',')[1];
        const bytes = Uint8Array.from(atob(encoded), char => char.charCodeAt(0));
        const stream = new ReadableStream({
          start(controller) {
            controller.enqueue(bytes);
            controller.close();
          },
        });
        const recovered = await new Response(
          stream.pipeThrough(new DecompressionStream('gzip'))
        ).arrayBuffer();
        expect(Array.from(new Uint8Array(recovered))).toEqual(Array.from(original));
        expect(result.attachment.originalSizeBytes).toBe(original.length);
      }
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it('accepts audio originals without transcribing them', async () => {
    const result = await validateAndReadFile(makeFile('voice.mp3', 'audio/mpeg', 3), 0);
    expect('attachment' in result && result.attachment.kind).toBe('file');
  });

  it('rejects when at max image count', async () => {
    const file = makeFile('x.png', 'image/png');
    const result = await validateAndReadFile(file, ATTACHMENT_MAX_IMAGES);
    expect('error' in result).toBe(true);
    if ('error' in result) {
      expect(result.error.code).toBe('too_many');
    }
  });

  it('accepts arbitrary MIME types', async () => {
    const file = makeFile('vector.svg', 'image/svg+xml');
    const result = await validateAndReadFile(file, 0);
    expect('attachment' in result && result.attachment.kind).toBe('file');
  });

  it('accepts Office documents and archives as original files', async () => {
    for (const mime of [
      'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
      'application/vnd.openxmlformats-officedocument.presentationml.presentation',
      'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
      'application/zip',
    ]) {
      const result = await validateAndReadFile(makeFile('f', mime, 8), 0);
      expect('attachment' in result && result.attachment.kind).toBe('file');
    }
  });

  it('accepts the text-extractable document formats (pdf/txt/markdown)', async () => {
    for (const mime of ['application/pdf', 'text/plain', 'text/markdown']) {
      const result = await validateAndReadFile(makeFile('f', mime, 8), 0);
      expect('attachment' in result && result.attachment.kind === 'file').toBe(true);
    }
  });

  it('accepts an image even for a non-vision model', async () => {
    const result = await validateAndReadFile(makeFile('p.png', 'image/png', 8), 0, 0, false);
    expect('attachment' in result && result.attachment.kind).toBe('image');
  });

  it('still accepts a document when allowImages is false', async () => {
    const result = await validateAndReadFile(makeFile('d.pdf', 'application/pdf', 8), 0, 0, false);
    expect('attachment' in result && result.attachment.kind === 'file').toBe(true);
  });

  it('accepts an image when allowImages is true (vision model)', async () => {
    const result = await validateAndReadFile(makeFile('p.png', 'image/png', 8), 0, 0, true);
    expect('attachment' in result && result.attachment.kind === 'image').toBe(true);
  });

  it('rejects files that exceed the file size limit', async () => {
    const oversizedFile = makeFile(
      'big.pdf',
      'application/pdf',
      ATTACHMENT_MAX_FILE_SIZE_BYTES + 1
    );
    const result = await validateAndReadFile(oversizedFile, 0);
    expect('error' in result).toBe(true);
    if ('error' in result) {
      expect(result.error.code).toBe('too_large');
    }
  });

  it('rejects files that exceed the size limit', async () => {
    const oversizedFile = makeFile('big.png', 'image/png', ATTACHMENT_MAX_IMAGE_SIZE_BYTES + 1);
    const result = await validateAndReadFile(oversizedFile, 0);
    expect('error' in result).toBe(true);
    if ('error' in result) {
      expect(result.error.code).toBe('too_large');
    }
  });

  it('returns an attachment for a valid image', async () => {
    const file = makeFile('ok.png', 'image/png', 512);
    const result = await validateAndReadFile(file, 0);
    expect('attachment' in result).toBe(true);
    if ('attachment' in result) {
      expect(result.attachment.mimeType).toBe('image/png');
      expect(result.attachment.kind).toBe('image');
      expect(result.attachment.dataUri).toMatch(/^data:image\/png;name=ok.png;base64,/);
      expect(result.attachment.file).toBe(file);
      expect(result.attachment.compressed).toBe(false);
    }
  });

  it('returns an attachment for a supported file', async () => {
    const file = makeFile('doc.pdf', 'application/pdf', 512);
    const result = await validateAndReadFile(file, 0);
    expect('attachment' in result).toBe(true);
    if ('attachment' in result) {
      expect(result.attachment.mimeType).toBe('application/pdf');
      expect(result.attachment.kind).toBe('file');
      expect(result.attachment.dataUri).toMatch(/^data:application\/pdf;name=doc.pdf;base64,/);
    }
  });

  it('returns read_failed when FileReader errors', async () => {
    const file = makeFile('bad.png', 'image/png', 1);
    const origFileReader = globalThis.FileReader;
    class FailingReader {
      onload: (() => void) | null = null;
      onerror: ((e: unknown) => void) | null = null;
      readAsDataURL() {
        setTimeout(() => this.onerror?.(new Error('read error')), 0);
      }
    }
    globalThis.FileReader = FailingReader as unknown as typeof FileReader;
    try {
      const result = await validateAndReadFile(file, 0);
      expect('error' in result).toBe(true);
      if ('error' in result) {
        expect(result.error.code).toBe('read_failed');
      }
    } finally {
      globalThis.FileReader = origFileReader;
    }
  });
});

describe('video attachments', () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('classifies video MIME types as the video kind', () => {
    expect(isVideoMimeType('video/mp4')).toBe(true);
    expect(isVideoMimeType('video/quicktime')).toBe(true);
    expect(isVideoMimeType('video/webm')).toBe(true);
    expect(isVideoMimeType('image/png')).toBe(false);
    expect(attachmentKindForMime('video/mp4')).toBe('video');
  });

  it('budgets originals rather than video preview frames', async () => {
    expect(imageMarkerCost('image')).toBe(1);
    expect(imageMarkerCost('video')).toBe(0);
    expect(imageMarkerCost('file')).toBe(0);
    const result = await validateAndReadFile(
      makeFile('clip.mp4', 'video/mp4', 8),
      ATTACHMENT_MAX_IMAGES,
      0,
      false
    );
    expect('attachment' in result && result.attachment.kind).toBe('video');
  });

  it('rejects video over the original file size limit', async () => {
    const result = await validateAndReadFile(
      makeFile('big.mp4', 'video/mp4', ATTACHMENT_MAX_VIDEO_SIZE_BYTES + 1),
      0
    );
    expect('error' in result && result.error.code).toBe('too_large');
    expect(ATTACHMENT_MAX_VIDEO_SIZE_BYTES).toBe(ATTACHMENT_MAX_FILE_SIZE_BYTES);
  });

  it('rejects video when the file count is full', async () => {
    const result = await validateAndReadFile(makeFile('clip.mp4', 'video/mp4', 8), 0, 4);
    expect('error' in result && result.error.code).toBe('too_many');
  });

  it('retains the exact video bytes without requiring a browser decoder', async () => {
    const decode = vi
      .spyOn(videoFrameExtractor, 'extract')
      .mockRejectedValue(new Error('codec unavailable'));
    const file = new File([Uint8Array.of(1, 2, 3, 4)], 'clip.mp4', { type: 'video/mp4' });
    const result = await validateAndReadFile(file, 0);
    expect('attachment' in result).toBe(true);
    if ('attachment' in result) {
      expect(result.attachment.dataUri).toBe('data:video/mp4;name=clip.mp4;base64,AQIDBA==');
      expect(attachmentsToParts([result.attachment])).toEqual([
        {
          kind: 'video',
          caption: null,
          durationMs: null,
          thumbnail: null,
          media: expect.objectContaining({
            url: 'data:video/mp4;name=clip.mp4;base64,AQIDBA==',
            contentType: 'video/mp4',
            fileName: 'clip.mp4',
          }),
        },
      ]);
    }
    expect(decode).not.toHaveBeenCalled();
  });
});

describe('extractVideoFrames (DOM-mocked)', () => {
  // jsdom has no video codec, so stub <video>/<canvas> + URL to exercise the
  // real decode/seek/paint path deterministically.
  let realCreateObjectURL: typeof URL.createObjectURL;
  let realRevokeObjectURL: typeof URL.revokeObjectURL;

  function makeFakeVideo(duration: number) {
    const listeners: Record<string, Array<() => void>> = {};
    let ct = 0;
    return {
      muted: false,
      preload: '',
      _src: '',
      duration,
      videoWidth: 320,
      videoHeight: 240,
      onloadedmetadata: null as null | (() => void),
      onerror: null as null | (() => void),
      set src(v: string) {
        this._src = v;
        // Fire metadata asynchronously after the impl assigns the handler.
        void Promise.resolve().then(() => this.onloadedmetadata?.());
      },
      get src() {
        return this._src;
      },
      get currentTime() {
        return ct;
      },
      set currentTime(v: number) {
        ct = v;
        (listeners['seeked'] ?? []).forEach(cb => cb());
      },
      addEventListener(ev: string, cb: () => void) {
        (listeners[ev] ??= []).push(cb);
      },
      removeEventListener(ev: string, cb: () => void) {
        listeners[ev] = (listeners[ev] ?? []).filter(f => f !== cb);
      },
      removeAttribute() {},
    };
  }

  function makeFakeCanvas() {
    return {
      width: 0,
      height: 0,
      getContext: () => ({ drawImage() {} }),
      toDataURL: () => 'data:image/jpeg;base64,FRAME',
    };
  }

  function installDom(duration: number) {
    const realCreate = document.createElement.bind(document);
    vi.spyOn(document, 'createElement').mockImplementation((tag: string) => {
      if (tag === 'video') return makeFakeVideo(duration) as unknown as HTMLElement;
      if (tag === 'canvas') return makeFakeCanvas() as unknown as HTMLElement;
      return realCreate(tag as keyof HTMLElementTagNameMap);
    });
  }

  beforeEach(() => {
    realCreateObjectURL = URL.createObjectURL;
    realRevokeObjectURL = URL.revokeObjectURL;
    URL.createObjectURL = vi.fn(() => 'blob:fake');
    URL.revokeObjectURL = vi.fn();
  });

  afterEach(() => {
    vi.restoreAllMocks();
    URL.createObjectURL = realCreateObjectURL;
    URL.revokeObjectURL = realRevokeObjectURL;
  });

  it('samples one frame per seek point for a normal clip', async () => {
    installDom(10);
    const frames = await extractVideoFrames(makeFile('c.mp4', 'video/mp4', 8), 2);
    expect(frames).toEqual(['data:image/jpeg;base64,FRAME', 'data:image/jpeg;base64,FRAME']);
  });

  it('handles a zero-duration clip without hanging (seek short-circuits)', async () => {
    installDom(0);
    const frames = await extractVideoFrames(makeFile('z.mp4', 'video/mp4', 8), 2);
    expect(frames.length).toBe(2);
  });

  it('the explicit preview helper rejects a video that fails to decode', async () => {
    const realCreate = document.createElement.bind(document);
    vi.spyOn(document, 'createElement').mockImplementation((tag: string) => {
      if (tag === 'video') {
        const v = makeFakeVideo(10);
        // Override src to fire onerror instead of onloadedmetadata.
        Object.defineProperty(v, 'src', {
          set(value: string) {
            this._src = value;
            void Promise.resolve().then(() => this.onerror?.());
          },
          get() {
            return this._src;
          },
        });
        return v as unknown as HTMLElement;
      }
      if (tag === 'canvas') return makeFakeCanvas() as unknown as HTMLElement;
      return realCreate(tag as keyof HTMLElementTagNameMap);
    });
    await expect(extractVideoFrames(makeFile('bad.mp4', 'video/mp4', 8))).rejects.toThrow();
  });
});

describe('attachmentsToParts', () => {
  it('returns no parts when there are no attachments', () => {
    expect(attachmentsToParts([])).toEqual([]);
  });

  it('sends the original video even when optional preview frames exist', () => {
    const video = makeAttachment({
      kind: 'video',
      file: makeFile('clip.mp4', 'video/mp4'),
      mimeType: 'video/mp4',
      dataUri: 'data:video/mp4;base64,original',
      previewUri: 'data:image/jpeg;base64,f1',
      frames: ['data:image/jpeg;base64,f1', 'data:image/jpeg;base64,f2'],
    });
    const [part] = attachmentsToParts([video]);
    expect(part.kind).toBe('video');
    expect(part).toMatchObject({ media: { url: 'data:video/mp4;base64,original' } });
  });

  it('maps an image to a typed image part carrying name, type and size', () => {
    const a = makeAttachment({
      dataUri: 'data:image/png;base64,abc',
      file: makeFile('shot.png', 'image/png'),
      mimeType: 'image/png',
      originalSizeBytes: 321,
    });
    expect(attachmentsToParts([a])).toEqual([
      {
        kind: 'image',
        altText: null,
        caption: null,
        media: {
          kind: 'image',
          url: 'data:image/png;base64,abc',
          contentType: 'image/png',
          fileName: 'shot.png',
          sizeBytes: 321,
          codec: null,
          durationMs: null,
          height: null,
          localCachePath: null,
          path: null,
          platformMessageId: null,
          transcribed: false,
          width: null,
        },
      },
    ]);
  });

  it('keeps the order of multiple attachments', () => {
    const a1 = makeAttachment({ id: '1', dataUri: 'data:image/png;base64,a' });
    const a2 = makeAttachment({ id: '2', dataUri: 'data:image/jpeg;base64,b' });
    const urls = attachmentsToParts([a1, a2]).map(p => (p as { media: { url: string } }).media.url);
    expect(urls).toEqual(['data:image/png;base64,a', 'data:image/jpeg;base64,b']);
  });

  it('maps non-image attachments to file parts with the file name', () => {
    const a = makeAttachment({
      kind: 'file',
      file: makeFile('doc.pdf', 'application/pdf'),
      dataUri: 'data:application/pdf;base64,abc',
      mimeType: 'application/pdf',
    });
    expect(attachmentsToParts([a])).toEqual([
      {
        kind: 'file',
        fileName: 'doc.pdf',
        caption: null,
        media: expect.objectContaining({
          kind: 'document',
          url: 'data:application/pdf;base64,abc',
          fileName: 'doc.pdf',
        }),
      },
    ]);
  });

  it('never emits upload markers', () => {
    const a = makeAttachment({ dataUri: 'data:image/png;base64,abc' });
    expect(JSON.stringify(attachmentsToParts([a]))).not.toMatch(/\[(IMAGE|FILE):/);
  });
});

describe('parseMessageImages', () => {
  it('returns empty dataUris and original text when no markers', () => {
    const result = parseMessageImages('hello world');
    expect(result.text).toBe('hello world');
    expect(result.dataUris).toEqual([]);
  });

  it('extracts a single image marker and strips it from text', () => {
    const result = parseMessageImages('describe this [IMAGE:data:image/png;base64,abc]');
    expect(result.text).toBe('describe this');
    expect(result.dataUris).toEqual(['data:image/png;base64,abc']);
  });

  it('extracts multiple image markers', () => {
    const result = parseMessageImages(
      '[IMAGE:data:image/png;base64,a] [IMAGE:data:image/jpeg;base64,b]'
    );
    expect(result.text).toBe('');
    expect(result.dataUris).toEqual(['data:image/png;base64,a', 'data:image/jpeg;base64,b']);
  });

  it('returns empty text when message is marker-only', () => {
    const result = parseMessageImages('[IMAGE:data:image/png;base64,xyz]');
    expect(result.text).toBe('');
    expect(result.dataUris).toHaveLength(1);
  });

  it('strips FILE markers without including them in dataUris', () => {
    const result = parseMessageImages('read this [FILE:data:application/pdf;base64,xyz]');
    expect(result.text).toBe('read this');
    expect(result.dataUris).toEqual([]);
  });

  it('handles mixed IMAGE and FILE markers', () => {
    const result = parseMessageImages(
      'check this [IMAGE:data:image/png;base64,a] and [FILE:data:application/pdf;base64,b] thanks'
    );
    expect(result.text).toBe('check this and thanks');
    expect(result.dataUris).toEqual(['data:image/png;base64,a']);
  });

  it('strips multiple FILE markers', () => {
    const result = parseMessageImages(
      '[FILE:data:application/pdf;base64,a] [FILE:data:text/plain;base64,b]'
    );
    expect(result.text).toBe('');
    expect(result.dataUris).toEqual([]);
  });

  it('preserves paragraph breaks (double newlines) in user text', () => {
    const result = parseMessageImages('first paragraph\n\nsecond paragraph');
    expect(result.text).toBe('first paragraph\n\nsecond paragraph');
  });

  it('preserves newlines when stripping a marker', () => {
    const result = parseMessageImages(
      'first paragraph\n\n[FILE:data:application/pdf;base64,x]\n\nsecond paragraph'
    );
    expect(result.text).toBe('first paragraph\n\n\n\nsecond paragraph');
    expect(result.text).toContain('\n\n');
  });
});

describe('formatFileSize', () => {
  it('formats bytes', () => {
    expect(formatFileSize(500)).toBe('500 B');
  });

  it('formats kilobytes', () => {
    expect(formatFileSize(1536)).toBe('1.5 KB');
  });

  it('formats megabytes', () => {
    expect(formatFileSize(2 * 1024 * 1024)).toBe('2.0 MB');
  });
});
