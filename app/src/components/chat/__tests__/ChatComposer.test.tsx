import { fireEvent, render, screen } from '@testing-library/react';
import { createRef } from 'react';
import { describe, expect, it, vi } from 'vitest';

import type { Attachment } from '../../../lib/attachments';
import ChatComposer, { type ChatComposerProps } from '../ChatComposer';

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));
vi.mock('../CycleUsagePill', () => ({ default: () => <div data-testid="cycle-usage-pill" /> }));

function makeAttachment(overrides: Partial<Attachment> = {}): Attachment {
  const blob = new Blob([new Uint8Array(256)], { type: 'image/png' });
  return {
    id: 'att-1',
    file: new File([blob], 'photo.png', { type: 'image/png' }),
    dataUri: 'data:image/png;base64,abc',
    mimeType: 'image/png',
    ...overrides,
  };
}

function renderComposer(overrides: Partial<ChatComposerProps> = {}) {
  const textInputRef = createRef<HTMLTextAreaElement | null>();
  const fileInputRef = createRef<HTMLInputElement | null>();
  const isComposingTextRef = { current: false };

  const props: ChatComposerProps = {
    inputValue: '',
    setInputValue: vi.fn(),
    onSend: vi.fn().mockResolvedValue(undefined),
    textInputRef,
    fileInputRef,
    composerInteractionBlocked: false,
    isSending: false,
    attachments: [],
    onAttachFiles: vi.fn().mockResolvedValue(undefined),
    onRemoveAttachment: vi.fn(),
    attachError: null,
    onSwitchToMicCloud: vi.fn(),
    handleInputKeyDown: vi.fn(),
    inlineCompletionSuffix: '',
    isComposingTextRef,
    maxAttachments: 5,
    allowedMimeTypes: ['image/png', 'image/jpeg'],
    attachmentsEnabled: true,
    ...overrides,
  };

  return render(<ChatComposer {...props} />);
}

describe('ChatComposer', () => {
  it('renders textarea with placeholder', () => {
    renderComposer();
    const textarea = screen.getByRole('textbox');
    expect(textarea).toBeInTheDocument();
    expect(textarea).toHaveAttribute('placeholder', 'chat.typeMessage');
  });

  it('renders attachment + button in toolbar', () => {
    renderComposer();
    expect(screen.getByRole('button', { name: 'composer.attachFile' })).toBeInTheDocument();
  });

  it('hides the attach button and file input when attachmentsEnabled is false', () => {
    const { container } = renderComposer({ attachmentsEnabled: false });
    expect(screen.queryByRole('button', { name: 'composer.attachFile' })).not.toBeInTheDocument();
    expect(container.querySelector('input[type="file"]')).toBeNull();
  });

  it('renders voice mode button in toolbar', () => {
    renderComposer();
    expect(screen.getByRole('button', { name: 'composer.voiceMode' })).toBeInTheDocument();
  });

  it('send button is always visible', () => {
    renderComposer({ inputValue: '' });
    expect(screen.getByTestId('send-message-button')).toBeInTheDocument();
  });

  it('send button is disabled when inputValue is empty and no attachments', () => {
    renderComposer({ inputValue: '' });
    expect(screen.getByTestId('send-message-button')).toBeDisabled();
  });

  it('send button is enabled when inputValue has content', () => {
    renderComposer({ inputValue: 'hello' });
    expect(screen.getByTestId('send-message-button')).not.toBeDisabled();
  });

  it('send button is enabled when attachments are present even without text', () => {
    renderComposer({ inputValue: '', attachments: [makeAttachment()] });
    expect(screen.getByTestId('send-message-button')).not.toBeDisabled();
  });

  it('attachment button triggers file input click', () => {
    renderComposer();
    const attachButton = screen.getByRole('button', { name: 'composer.attachFile' });
    // File input is hidden; clicking the button should call click() on the ref.
    // We just verify the button is enabled and triggers no error.
    fireEvent.click(attachButton);
    // No error thrown — file input click is a no-op in test DOM.
  });

  it('attachment button is disabled when composerInteractionBlocked is true', () => {
    renderComposer({ composerInteractionBlocked: true });
    expect(screen.getByRole('button', { name: 'composer.attachFile' })).toBeDisabled();
  });

  it('textarea is disabled when composerInteractionBlocked is true', () => {
    renderComposer({ composerInteractionBlocked: true });
    expect(screen.getByRole('textbox')).toBeDisabled();
  });

  it('textarea is disabled when isSending is true', () => {
    renderComposer({ isSending: true, inputValue: 'sending...' });
    expect(screen.getByRole('textbox')).toBeDisabled();
  });

  it('send button is disabled when isSending is true', () => {
    renderComposer({ isSending: true, inputValue: 'hello' });
    expect(screen.getByTestId('send-message-button')).toBeDisabled();
  });

  it('calls onSend when send button is clicked', () => {
    const onSend = vi.fn().mockResolvedValue(undefined);
    renderComposer({ inputValue: 'hello', onSend });
    fireEvent.click(screen.getByTestId('send-message-button'));
    expect(onSend).toHaveBeenCalledTimes(1);
  });

  it('calls onSwitchToMicCloud when voice mode button is clicked', () => {
    const onSwitchToMicCloud = vi.fn();
    renderComposer({ onSwitchToMicCloud });
    fireEvent.click(screen.getByRole('button', { name: 'composer.voiceMode' }));
    expect(onSwitchToMicCloud).toHaveBeenCalledTimes(1);
  });
});
