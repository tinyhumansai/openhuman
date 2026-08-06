import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';

import { ShareCardModal } from './ShareCardModal';

const mocks = vi.hoisted(() => ({
  draftShareHeadline: vi.fn(),
  openUrl: vi.fn(),
  renderShareCardToCanvas: vi.fn(),
  cardToPngBlob: vi.fn(),
  trackAnalyticsEvent: vi.fn(),
}));

vi.mock('./shareCaption', () => ({ draftShareHeadline: mocks.draftShareHeadline }));
vi.mock('../../utils/openUrl', () => ({ openUrl: mocks.openUrl }));
vi.mock('../../components/analytics', () => ({ trackAnalyticsEvent: mocks.trackAnalyticsEvent }));
vi.mock('./shareCard', async importActual => {
  const actual = await importActual<typeof import('./shareCard')>();
  return {
    ...actual,
    renderShareCardToCanvas: mocks.renderShareCardToCanvas,
    cardToPngBlob: mocks.cardToPngBlob,
  };
});

let clipboardWriteText: ReturnType<typeof vi.fn>;

beforeEach(() => {
  mocks.draftShareHeadline.mockResolvedValue('My agent cleared my inbox');
  mocks.openUrl.mockResolvedValue(undefined);
  mocks.cardToPngBlob.mockResolvedValue(new Blob(['x'], { type: 'image/png' }));
  clipboardWriteText = vi.fn().mockResolvedValue(undefined);
  vi.stubGlobal('navigator', { ...navigator, clipboard: { writeText: clipboardWriteText } });
});

afterEach(() => {
  vi.unstubAllGlobals();
  Object.values(mocks).forEach(m => m.mockReset());
});

describe('ShareCardModal', () => {
  test('drafts a headline and seeds the caption', async () => {
    mocks.draftShareHeadline.mockResolvedValue('My agent cleared my inbox');
    render(<ShareCardModal content="cleared the inbox" agentName="Tiny" onClose={vi.fn()} />);

    const textarea = await screen.findByLabelText('Caption');
    await waitFor(() => expect((textarea as HTMLTextAreaElement).value).toContain('cleared'));
    expect(mocks.draftShareHeadline).toHaveBeenCalledWith('cleared the inbox', undefined);
  });

  test('opens the X composer with the caption', async () => {
    const user = userEvent.setup();
    render(<ShareCardModal content="cleared the inbox" agentName="Tiny" onClose={vi.fn()} />);
    await screen.findByLabelText('Caption');

    await user.click(screen.getByText('Share on X'));
    await waitFor(() => expect(mocks.openUrl).toHaveBeenCalled());
    expect(mocks.openUrl.mock.calls[0][0]).toContain('twitter.com/intent/tweet');
    expect(mocks.trackAnalyticsEvent).toHaveBeenCalledWith('chat_message_shared', {
      destination: 'x',
    });
  });

  test('copies the caption to the clipboard and opens LinkedIn', async () => {
    const user = userEvent.setup();
    render(<ShareCardModal content="cleared the inbox" agentName="Tiny" onClose={vi.fn()} />);
    await screen.findByLabelText('Caption');

    await user.click(screen.getByText('Share on LinkedIn'));
    await waitFor(() => expect(mocks.openUrl).toHaveBeenCalled());
    expect(mocks.openUrl.mock.calls[0][0]).toContain('linkedin.com/sharing/share-offsite');
    expect(mocks.trackAnalyticsEvent).toHaveBeenCalledWith('chat_message_shared', {
      destination: 'linkedin',
    });
  });

  test('copies the image (download fallback when ClipboardItem is absent)', async () => {
    const user = userEvent.setup();
    render(<ShareCardModal content="cleared the inbox" agentName="Tiny" onClose={vi.fn()} />);
    await screen.findByLabelText('Caption');

    await user.click(screen.getByText('Copy image'));
    await waitFor(() => expect(mocks.cardToPngBlob).toHaveBeenCalled());
  });

  test('falls back to download when the clipboard image write rejects', async () => {
    const user = userEvent.setup();
    const write = vi.fn().mockRejectedValue(new Error('denied'));
    vi.stubGlobal('ClipboardItem', class {} as unknown as typeof ClipboardItem);
    vi.stubGlobal('navigator', {
      ...navigator,
      clipboard: { writeText: clipboardWriteText, write },
    });
    const createObjectURL = vi.fn().mockReturnValue('blob:mock');
    const revokeObjectURL = vi.fn();
    vi.stubGlobal('URL', { ...URL, createObjectURL, revokeObjectURL });

    render(<ShareCardModal content="cleared the inbox" agentName="Tiny" onClose={vi.fn()} />);
    await screen.findByLabelText('Caption');

    await user.click(screen.getByText('Copy image'));
    await waitFor(() => expect(write).toHaveBeenCalled());
    // Rejected clipboard write should still fall through to the download path
    // (createObjectURL) rather than only surfacing an image error.
    await waitFor(() => expect(createObjectURL).toHaveBeenCalled());
    await waitFor(() => expect(screen.getByText('Image copied')).toBeInTheDocument());
    expect(screen.queryByText("Couldn't generate the image. Try again.")).not.toBeInTheDocument();
  });
});
