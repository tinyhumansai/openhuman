import { describe, expect, it } from 'vitest';

import {
  type Attachment,
  ATTACHMENT_MAX_IMAGES,
  ATTACHMENT_MAX_SIZE_BYTES,
  buildMessageWithAttachments,
  fileToDataUri,
  formatFileSize,
  isAllowedMimeType,
  parseMessageImages,
  validateAndReadFile,
} from './attachments';

function makeFile(name: string, type: string, size = 1024): File {
  const blob = new Blob([new Uint8Array(size)], { type });
  return new File([blob], name, { type });
}

function makeAttachment(overrides: Partial<Attachment> = {}): Attachment {
  return {
    id: 'test-id',
    file: makeFile('test.png', 'image/png'),
    dataUri: 'data:image/png;base64,abc',
    mimeType: 'image/png',
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
    const file = makeFile('doc.pdf', 'application/pdf');
    const result = await validateAndReadFile(file, 0);
    expect('error' in result).toBe(true);
    if ('error' in result) {
      expect(result.error.code).toBe('unsupported_type');
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
      expect(result.attachment.dataUri).toMatch(/^data:image\/png;base64,/);
      expect(result.attachment.file).toBe(file);
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

describe('buildMessageWithAttachments', () => {
  it('returns text unchanged when no attachments', () => {
    expect(buildMessageWithAttachments('hello', [])).toBe('hello');
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
