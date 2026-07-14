import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  ALLOWED_ATTACHMENT_MIME_TYPES,
  type Attachment,
  ATTACHMENT_MAX_FILE_SIZE_BYTES,
  ATTACHMENT_MAX_IMAGES,
  ATTACHMENT_MAX_SIZE_BYTES,
  ATTACHMENT_MAX_VIDEO_SIZE_BYTES,
  attachmentKindForMime,
  buildMessageWithAttachments,
  extractVideoFrames,
  fileToDataUri,
  formatFileSize,
  imageMarkerCost,
  isAllowedMimeType,
  isVideoMimeType,
  parseMessageImages,
  validateAndReadFile,
  VIDEO_FRAME_COUNT,
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
  it('rejects when at max image count', async () => {
    const file = makeFile('x.png', 'image/png');
    const result = await validateAndReadFile(file, ATTACHMENT_MAX_IMAGES);
    expect('error' in result).toBe(true);
    if ('error' in result) {
      expect(result.error.code).toBe('too_many');
    }
  });

  it('rejects unsupported MIME types', async () => {
    const file = makeFile('vector.svg', 'image/svg+xml');
    const result = await validateAndReadFile(file, 0);
    expect('error' in result).toBe(true);
    if ('error' in result) {
      expect(result.error.code).toBe('unsupported_type');
    }
  });

  it('rejects non-extractable documents (docx/pptx/xlsx) the agent cannot read', async () => {
    for (const mime of [
      'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
      'application/vnd.openxmlformats-officedocument.presentationml.presentation',
      'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
      'application/zip',
    ]) {
      const result = await validateAndReadFile(makeFile('f', mime, 8), 0);
      expect('error' in result && result.error.code).toBe('unsupported_type');
    }
  });

  it('accepts the text-extractable document formats (pdf/txt/markdown)', async () => {
    for (const mime of ['application/pdf', 'text/plain', 'text/markdown']) {
      const result = await validateAndReadFile(makeFile('f', mime, 8), 0);
      expect('attachment' in result && result.attachment.kind === 'file').toBe(true);
    }
  });

  it('rejects an image when allowImages is false (non-vision model)', async () => {
    const result = await validateAndReadFile(makeFile('p.png', 'image/png', 8), 0, 0, false);
    expect('error' in result && result.error.code).toBe('image_not_supported');
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
    const oversizedFile = makeFile('big.png', 'image/png', ATTACHMENT_MAX_SIZE_BYTES + 1);
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
      expect(result.attachment.dataUri).toMatch(/^data:image\/png;base64,/);
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
      expect(result.attachment.dataUri).toMatch(/^data:application\/pdf;base64,/);
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

  it('costs 1 image marker per image and VIDEO_FRAME_COUNT per video', () => {
    expect(imageMarkerCost('image')).toBe(1);
    expect(imageMarkerCost('video')).toBe(VIDEO_FRAME_COUNT);
    expect(imageMarkerCost('file')).toBe(0);
  });

  it('rejects video when allowImages is false (non-vision model)', async () => {
    const result = await validateAndReadFile(makeFile('clip.mp4', 'video/mp4', 8), 0, 0, false);
    expect('error' in result && result.error.code).toBe('video_not_supported');
  });

  it('rejects video over the video size limit', async () => {
    const big = makeFile('big.mp4', 'video/mp4', ATTACHMENT_MAX_VIDEO_SIZE_BYTES + 1);
    const result = await validateAndReadFile(big, 0, 0, true);
    expect('error' in result && result.error.code).toBe('too_large');
  });

  it('rejects video when its frames would exceed the shared image-marker budget', async () => {
    // Budget full of images already → no room for a video's frames.
    const result = await validateAndReadFile(
      makeFile('clip.mp4', 'video/mp4', 8),
      ATTACHMENT_MAX_IMAGES,
      0,
      true
    );
    expect('error' in result && result.error.code).toBe('too_many');
    // Reported as the image budget (frames are images to the model).
    expect('error' in result && result.error.code === 'too_many' && result.error.kind).toBe(
      'image'
    );
  });

  it('rejects a video that partially overflows the budget (images + frames)', async () => {
    // One image used (1 marker); a video costs VIDEO_FRAME_COUNT more. With the
    // default budget of 4, 1 + 2 = 3 fits, but seed close to the cap to overflow.
    const used = ATTACHMENT_MAX_IMAGES - (VIDEO_FRAME_COUNT - 1); // leaves < cost
    const result = await validateAndReadFile(makeFile('c.mp4', 'video/mp4', 8), used, 0, true);
    expect('error' in result && result.error.code).toBe('too_many');
  });

  it('samples frames and returns a video attachment within budget', async () => {
    const frames = ['data:image/jpeg;base64,f1', 'data:image/jpeg;base64,f2'];
    vi.spyOn(videoFrameExtractor, 'extract').mockResolvedValue(frames);
    const file = makeFile('clip.mp4', 'video/mp4', 4096);
    const result = await validateAndReadFile(file, 0, 0, true);
    expect('attachment' in result).toBe(true);
    if ('attachment' in result) {
      expect(result.attachment.kind).toBe('video');
      expect(result.attachment.frames).toEqual(frames);
      expect(result.attachment.previewUri).toBe(frames[0]);
      expect(result.attachment.file).toBe(file);
    }
  });

  it('fails when no frames could be extracted', async () => {
    vi.spyOn(videoFrameExtractor, 'extract').mockResolvedValue([]);
    const result = await validateAndReadFile(makeFile('clip.mp4', 'video/mp4', 8), 0, 0, true);
    expect('error' in result && result.error.code).toBe('read_failed');
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

  it('propagates a read_failed error when the video fails to decode', async () => {
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
    const result = await validateAndReadFile(makeFile('bad.mp4', 'video/mp4', 8), 0, 0, true);
    expect('error' in result && result.error.code).toBe('read_failed');
  });
});

describe('buildMessageWithAttachments', () => {
  it('returns text unchanged when no attachments', () => {
    expect(buildMessageWithAttachments('hello', [])).toBe('hello');
  });

  it('expands a video into one IMAGE marker per sampled frame', () => {
    const video = makeAttachment({
      kind: 'video',
      file: makeFile('clip.mp4', 'video/mp4'),
      mimeType: 'video/mp4',
      dataUri: 'data:image/jpeg;base64,f1',
      previewUri: 'data:image/jpeg;base64,f1',
      frames: ['data:image/jpeg;base64,f1', 'data:image/jpeg;base64,f2'],
    });
    const result = buildMessageWithAttachments('watch', [video]);
    expect(result).toBe(
      'watch [IMAGE:data:image/jpeg;base64,f1] [IMAGE:data:image/jpeg;base64,f2]'
    );
  });

  it('appends IMAGE markers after the text', () => {
    const a = makeAttachment({ dataUri: 'data:image/png;base64,abc' });
    const result = buildMessageWithAttachments('describe this', [a]);
    expect(result).toBe('describe this [IMAGE:data:image/png;base64,abc]');
  });

  it('emits only markers when text is empty', () => {
    const a = makeAttachment({ dataUri: 'data:image/png;base64,abc' });
    const result = buildMessageWithAttachments('', [a]);
    expect(result).toBe('[IMAGE:data:image/png;base64,abc]');
  });

  it('handles multiple attachments', () => {
    const a1 = makeAttachment({ id: '1', dataUri: 'data:image/png;base64,a' });
    const a2 = makeAttachment({ id: '2', dataUri: 'data:image/jpeg;base64,b' });
    const result = buildMessageWithAttachments('look', [a1, a2]);
    expect(result).toBe('look [IMAGE:data:image/png;base64,a] [IMAGE:data:image/jpeg;base64,b]');
  });

  it('emits FILE markers for non-image attachments', () => {
    const a = makeAttachment({
      kind: 'file',
      file: makeFile('doc.pdf', 'application/pdf'),
      dataUri: 'data:application/pdf;base64,abc',
      mimeType: 'application/pdf',
    });
    const result = buildMessageWithAttachments('read this', [a]);
    expect(result).toBe('read this [FILE:data:application/pdf;base64,abc]');
  });

  it('trims leading/trailing whitespace from text', () => {
    const a = makeAttachment({ dataUri: 'data:image/png;base64,x' });
    const result = buildMessageWithAttachments('  hi  ', [a]);
    expect(result).toBe('hi [IMAGE:data:image/png;base64,x]');
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
