import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import type { ThreadMessage } from '../../../types/thread';
import { SAFE_IMAGE_DATA_URI_RE, UserTurnBody } from './UserTurnBody';

const PNG_DATA_URI =
  'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==';

function makeMessage(extraMetadata: Record<string, unknown> = {}): ThreadMessage {
  return {
    id: 'm-1',
    sender: 'user',
    type: 'text',
    content: 'A question?',
    extraMetadata,
    createdAt: '2026-01-01T00:00:00.000Z',
  } as ThreadMessage;
}

describe('SAFE_IMAGE_DATA_URI_RE', () => {
  it('accepts a well-formed base64 image data URI', () => {
    expect(SAFE_IMAGE_DATA_URI_RE.test(PNG_DATA_URI)).toBe(true);
  });

  it('rejects a non-image or script-bearing URI', () => {
    expect(SAFE_IMAGE_DATA_URI_RE.test('javascript:alert(1)')).toBe(false);
    expect(SAFE_IMAGE_DATA_URI_RE.test('data:text/html;base64,PHNjcmlwdD4=')).toBe(false);
  });
});

describe('UserTurnBody', () => {
  it('renders the turn text', () => {
    render(
      <UserTurnBody
        msg={makeMessage()}
        displayText="A question?"
        fallbackDataUris={[]}
        showTime={false}
      />
    );

    expect(screen.getByText('A question?')).toBeInTheDocument();
  });

  it('renders a timestamp only when asked for one', () => {
    const { container, rerender } = render(
      <UserTurnBody msg={makeMessage()} displayText="Hi" fallbackDataUris={[]} showTime={false} />
    );
    const withoutTime = container.textContent ?? '';

    rerender(<UserTurnBody msg={makeMessage()} displayText="Hi" fallbackDataUris={[]} showTime />);

    expect(withoutTime.trim()).toBe('Hi');
    expect((container.textContent ?? '').length).toBeGreaterThan(withoutTime.length);
  });

  it('renders file attachment chips from persisted metadata', () => {
    render(
      <UserTurnBody
        msg={makeMessage({
          attachmentKinds: ['file', 'video'],
          attachmentNames: ['spec.pdf', 'clip.mp4'],
        })}
        displayText=""
        fallbackDataUris={[]}
        showTime={false}
      />
    );

    expect(screen.getByText('spec.pdf')).toBeInTheDocument();
    expect(screen.getByText('clip.mp4')).toBeInTheDocument();
  });

  it('drops an attachment URI that is not a safe image data URI', () => {
    render(
      <UserTurnBody
        msg={makeMessage({ attachmentDataUris: ['javascript:alert(1)', PNG_DATA_URI] })}
        displayText=""
        fallbackDataUris={[]}
        showTime={false}
      />
    );

    // Exactly one image survives the filter, and its `src` is never the
    // rejected value.
    const images = screen.getAllByRole('presentation', { hidden: true });
    expect(images).toHaveLength(1);
    expect(images[0].getAttribute('src')).not.toContain('javascript:');
  });
});
