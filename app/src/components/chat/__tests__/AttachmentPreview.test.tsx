import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { Attachment } from '../../../lib/attachments';
import AttachmentPreview from '../AttachmentPreview';

function makeAttachment(overrides: Partial<Attachment> = {}): Attachment {
  const blob = new Blob([new Uint8Array(512)], { type: 'image/png' });
  return {
    id: 'att-1',
    kind: 'image',
    file: new File([blob], 'test.png', { type: 'image/png' }),
    dataUri: 'data:image/png;base64,abc',
    mimeType: 'image/png',
    originalSizeBytes: 512,
    payloadSizeBytes: 512,
    compressed: false,
    ...overrides,
  };
}

describe('AttachmentPreview', () => {
  it('renders nothing when attachments list is empty', () => {
    const { container } = render(<AttachmentPreview attachments={[]} onRemove={vi.fn()} />);
    expect(container.firstChild).toBeNull();
  });

  it('renders a chip with filename and file size for each attachment', () => {
    const att = makeAttachment();
    render(<AttachmentPreview attachments={[att]} onRemove={vi.fn()} />);
    expect(screen.getByText('test.png')).toBeInTheDocument();
    expect(screen.getByText('512 B')).toBeInTheDocument();
  });

  it('renders a thumbnail image with the dataUri as src', () => {
    const att = makeAttachment({ dataUri: 'data:image/png;base64,xyz' });
    render(<AttachmentPreview attachments={[att]} onRemove={vi.fn()} />);
    const img = screen.getByAltText('test.png') as HTMLImageElement;
    expect(img.src).toBe('data:image/png;base64,xyz');
  });

  it('renders a document icon for non-image files', () => {
    const file = new File([new Uint8Array(128)], 'doc.pdf', { type: 'application/pdf' });
    const att = makeAttachment({
      kind: 'file',
      file,
      dataUri: 'data:application/pdf;base64,abc',
      mimeType: 'application/pdf',
      originalSizeBytes: 128,
      payloadSizeBytes: 128,
    });
    render(<AttachmentPreview attachments={[att]} onRemove={vi.fn()} />);
    expect(screen.getByText('doc.pdf')).toBeInTheDocument();
    expect(screen.queryByAltText('doc.pdf')).not.toBeInTheDocument();
  });

  it('renders a video poster thumbnail with a play overlay', () => {
    const file = new File([new Uint8Array(64)], 'clip.mp4', { type: 'video/mp4' });
    const att = makeAttachment({
      kind: 'video',
      file,
      dataUri: 'data:image/jpeg;base64,poster',
      previewUri: 'data:image/jpeg;base64,poster',
      mimeType: 'video/mp4',
      frames: ['data:image/jpeg;base64,poster'],
    });
    const { container } = render(<AttachmentPreview attachments={[att]} onRemove={vi.fn()} />);
    const img = screen.getByAltText('clip.mp4') as HTMLImageElement;
    expect(img.src).toBe('data:image/jpeg;base64,poster');
    expect(screen.getByText('clip.mp4')).toBeInTheDocument();
    // Explicitly assert the play overlay (the ▶ triangle drawn over the poster)
    // is rendered — without this the test would still pass if the overlay
    // disappeared, which is exactly the regression the title promises to catch.
    expect(container.querySelector('path[d="M8 5v14l11-7z"]')).not.toBeNull();
  });

  it('calls onRemove with the attachment id when × is clicked', () => {
    const onRemove = vi.fn();
    const att = makeAttachment({ id: 'att-42' });
    render(<AttachmentPreview attachments={[att]} onRemove={onRemove} />);
    fireEvent.click(screen.getByRole('button', { name: /remove test\.png/i }));
    expect(onRemove).toHaveBeenCalledWith('att-42');
  });

  it('disables the remove button when disabled prop is true', () => {
    const att = makeAttachment();
    render(<AttachmentPreview attachments={[att]} onRemove={vi.fn()} disabled />);
    expect(screen.getByRole('button', { name: /remove test\.png/i })).toBeDisabled();
  });

  it('renders multiple chips', () => {
    const a1 = makeAttachment({ id: '1', file: new File([], 'a.png', { type: 'image/png' }) });
    const a2 = makeAttachment({
      id: '2',
      file: new File([], 'b.jpg', { type: 'image/jpeg' }),
      mimeType: 'image/jpeg',
    });
    render(<AttachmentPreview attachments={[a1, a2]} onRemove={vi.fn()} />);
    expect(screen.getByText('a.png')).toBeInTheDocument();
    expect(screen.getByText('b.jpg')).toBeInTheDocument();
  });
});
