import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { Attachment } from '../../../lib/attachments';
import AttachmentPreview from '../AttachmentPreview';

function makeAttachment(overrides: Partial<Attachment> = {}): Attachment {
  const blob = new Blob([new Uint8Array(512)], { type: 'image/png' });
  return {
    id: 'att-1',
    file: new File([blob], 'test.png', { type: 'image/png' }),
    dataUri: 'data:image/png;base64,abc',
    mimeType: 'image/png',
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
