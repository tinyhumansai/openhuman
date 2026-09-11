import {
  type AppendMessage,
  AssistantRuntimeProvider,
  type ThreadMessageLike,
  useExternalStoreRuntime,
} from '@assistant-ui/react';
import { fireEvent, render, screen } from '@testing-library/react';
import { createRef, type ReactNode, useEffect, useState } from 'react';
import { describe, expect, it, vi } from 'vitest';

import type { Attachment } from '../../../lib/attachments';
import ChatComposer, { type ChatComposerProps } from '../ChatComposer';

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

/**
 * `ChatComposer` is built on the headless `ComposerPrimitive`, which reads its
 * runtime from React context, so every render here needs one. The adapter is
 * deliberately inert — this suite asserts the composer's own behaviour, and the
 * composer never routes a send through the runtime (see the component's doc
 * comment for why). What the runtime supplies is the composer *text store* the
 * `inputValue` prop is bridged into.
 */
const EMPTY_RUNTIME_MESSAGES: ThreadMessageLike[] = [];

function Runtime({
  children,
  onNew = async () => {},
}: {
  children: ReactNode;
  onNew?: (message: AppendMessage) => Promise<void>;
}) {
  const runtime = useExternalStoreRuntime<ThreadMessageLike>({
    messages: EMPTY_RUNTIME_MESSAGES,
    isRunning: false,
    convertMessage: m => m,
    onNew,
  });
  return <AssistantRuntimeProvider runtime={runtime}>{children}</AssistantRuntimeProvider>;
}

function makeAttachment(overrides: Partial<Attachment> = {}): Attachment {
  const blob = new Blob([new Uint8Array(256)], { type: 'image/png' });
  return {
    id: 'att-1',
    kind: 'image',
    file: new File([blob], 'photo.png', { type: 'image/png' }),
    dataUri: 'data:image/png;base64,abc',
    mimeType: 'image/png',
    originalSizeBytes: 256,
    payloadSizeBytes: 256,
    compressed: false,
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

  return render(<ChatComposer {...props} />, { wrapper: Runtime });
}

/** A composer with real local text state, for cases a stubbed setter can't cover. */
function Harness({ onSend }: { onSend: (text?: string) => Promise<void> }) {
  const [value, setValue] = useState('hello');
  const textInputRef = createRef<HTMLTextAreaElement | null>();
  const fileInputRef = createRef<HTMLInputElement | null>();
  const isComposingTextRef = { current: false };
  return (
    <ChatComposer
      inputValue={value}
      setInputValue={v => setValue(typeof v === 'function' ? v(value) : v)}
      onSend={onSend}
      textInputRef={textInputRef}
      fileInputRef={fileInputRef}
      composerInteractionBlocked={false}
      isSending={false}
      attachments={[]}
      onAttachFiles={vi.fn().mockResolvedValue(undefined)}
      onRemoveAttachment={vi.fn()}
      attachError={null}
      onSwitchToMicCloud={vi.fn()}
      handleInputKeyDown={vi.fn()}
      inlineCompletionSuffix=""
      isComposingTextRef={isComposingTextRef}
      maxAttachments={0}
      allowedMimeTypes={[]}
      attachmentsEnabled={false}
    />
  );
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

  it('shows the stop button (not send) while sending with an empty composer', () => {
    renderComposer({ isSending: true, inputValue: '', onStopGeneration: vi.fn() });
    expect(screen.getByTestId('stop-generation-button')).toBeInTheDocument();
    expect(screen.queryByTestId('send-message-button')).not.toBeInTheDocument();
  });

  it('stop button stays enabled while sending', () => {
    renderComposer({ isSending: true, inputValue: '', onStopGeneration: vi.fn() });
    expect(screen.getByTestId('stop-generation-button')).not.toBeDisabled();
  });

  it('calls onStopGeneration when the stop button is clicked', () => {
    const onStopGeneration = vi.fn();
    renderComposer({ isSending: true, inputValue: '', onStopGeneration });
    fireEvent.click(screen.getByTestId('stop-generation-button'));
    expect(onStopGeneration).toHaveBeenCalledTimes(1);
  });

  it('reverts to the send button while sending once a follow-up is typed', () => {
    // Parallel/follow-up send: a typed follow-up should be queuable, so the
    // Send arrow returns instead of the Stop button.
    renderComposer({
      isSending: true,
      allowParallelSend: true,
      inputValue: 'a follow-up',
      onStopGeneration: vi.fn(),
    });
    expect(screen.queryByTestId('stop-generation-button')).not.toBeInTheDocument();
    expect(screen.getByTestId('send-message-button')).toBeInTheDocument();
  });

  it('falls back to the disabled send button while sending when no onStopGeneration', () => {
    renderComposer({ isSending: true, inputValue: '' });
    expect(screen.queryByTestId('stop-generation-button')).not.toBeInTheDocument();
    expect(screen.getByTestId('send-message-button')).toBeDisabled();
  });

  it('calls onSwitchToMicCloud when voice mode button is clicked', () => {
    const onSwitchToMicCloud = vi.fn();
    renderComposer({ onSwitchToMicCloud });
    fireEvent.click(screen.getByRole('button', { name: 'composer.voiceMode' }));
    expect(onSwitchToMicCloud).toHaveBeenCalledTimes(1);
  });

  describe('runtime boundary', () => {
    it('renders with no assistant-ui runtime in context at all', () => {
      // The primitives throw on a missing runtime; the boundary supplies an
      // inert one so a host that mounts none still gets a working composer
      // rather than a crash. Note the deliberate absence of `wrapper: Runtime`.
      render(
        <ChatComposer
          inputValue="standalone"
          setInputValue={vi.fn()}
          onSend={vi.fn().mockResolvedValue(undefined)}
          textInputRef={createRef<HTMLTextAreaElement | null>()}
          fileInputRef={createRef<HTMLInputElement | null>()}
          composerInteractionBlocked={false}
          isSending={false}
          attachments={[]}
          onAttachFiles={vi.fn().mockResolvedValue(undefined)}
          onRemoveAttachment={vi.fn()}
          attachError={null}
          onSwitchToMicCloud={vi.fn()}
          handleInputKeyDown={vi.fn()}
          inlineCompletionSuffix=""
          isComposingTextRef={{ current: false }}
          maxAttachments={0}
          allowedMimeTypes={[]}
          attachmentsEnabled={false}
        />
      );
      expect(screen.getByRole('textbox')).toHaveValue('standalone');
      expect(screen.getByTestId('send-message-button')).toBeInTheDocument();
    });
  });

  describe('assistant-ui composer text bridge', () => {
    it('renders the inputValue prop through the primitive input', () => {
      // The primitive is not a controlled React input — it renders the
      // runtime's composer text. The bridge is what makes the prop visible.
      renderComposer({ inputValue: 'from the host surface' });
      expect(screen.getByRole('textbox')).toHaveValue('from the host surface');
    });

    it('reports typing back to the host through setInputValue', () => {
      const setInputValue = vi.fn();
      renderComposer({ setInputValue });
      fireEvent.change(screen.getByRole('textbox'), { target: { value: 'typed' } });
      expect(setInputValue).toHaveBeenCalledWith('typed');
    });

    it('keeps the host authoritative when it ignores the change', () => {
      // A surface that does not adopt the change must not end up with a
      // composer showing text its own state never accepted.
      renderComposer({ inputValue: 'host value', setInputValue: vi.fn() });
      fireEvent.change(screen.getByRole('textbox'), { target: { value: 'ignored' } });
      expect(screen.getByRole('textbox')).toHaveValue('host value');
    });
  });

  describe('submit interception', () => {
    it('sends through onSend, not the runtime, when the form submits', () => {
      const onSend = vi.fn().mockResolvedValue(undefined);
      const onNew = vi.fn().mockResolvedValue(undefined);
      const { container } = render(<Harness onSend={onSend} />, {
        wrapper: ({ children }) => <Runtime onNew={onNew}>{children}</Runtime>,
      });
      fireEvent.submit(container.querySelector('form')!);
      expect(onSend).toHaveBeenCalledTimes(1);
      // The primitive's own submit handler would have called the runtime's
      // `onNew`, which cannot carry attachments or the parallel-send modifiers.
      expect(onNew).not.toHaveBeenCalled();
    });

    it('does not send on submit while the composer is locked', () => {
      const onSend = vi.fn().mockResolvedValue(undefined);
      const { container } = renderComposer({ onSend, composerInteractionBlocked: true });
      fireEvent.submit(container.querySelector('form')!);
      expect(onSend).not.toHaveBeenCalled();
    });
  });

  describe('Enter key ownership', () => {
    it('does not send when the host key handler declines to act', () => {
      // The regression this pins: assistant-ui's own Enter handler runs after
      // the host's (Radix composes them) and fires whenever the host merely
      // returns instead of calling preventDefault. Mid-IME-composition the host
      // declines and the keydown carries no `isComposing` flag, so the
      // primitive would submit a half-composed word. `submitMode="none"` is
      // what stops it.
      const onSend = vi.fn().mockResolvedValue(undefined);
      renderComposer({ inputValue: 'かな', onSend, handleInputKeyDown: vi.fn() });
      fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter', keyCode: 229 });
      expect(onSend).not.toHaveBeenCalled();
    });

    it('still routes Enter to the host key handler', () => {
      const handleInputKeyDown = vi.fn();
      renderComposer({ inputValue: 'hello', handleInputKeyDown });
      fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter' });
      expect(handleInputKeyDown).toHaveBeenCalledTimes(1);
    });
  });

  describe('mascotDock', () => {
    it('renders the node inside the input box, not the header stack', () => {
      // Anchoring matters: the dock is absolutely positioned against the input
      // box so it stands on its top edge. If it were hoisted into the header
      // stack it would drift whenever follow-ups or the goal editor open.
      const { container } = renderComposer({
        mascotDock: <div data-testid="mascot-dock" />,
        headerSlots: [<div key="hdr" data-testid="header-slot" />],
      });

      const dock = screen.getByTestId('mascot-dock');
      const inputBox = container.querySelector('.rounded-3xl.border');
      expect(inputBox).not.toBeNull();
      expect(inputBox!.contains(dock)).toBe(true);
      expect(inputBox!.contains(screen.getByTestId('header-slot'))).toBe(false);
    });

    it('renders nothing extra when omitted', () => {
      renderComposer();
      expect(screen.queryByTestId('mascot-dock')).not.toBeInTheDocument();
    });
  });

  describe('drag-and-drop', () => {
    function makeVideoFile() {
      return new File([new Uint8Array(8)], 'clip.mp4', { type: 'video/mp4' });
    }

    it('shows the drop overlay while a file drag is over the composer', () => {
      renderComposer();
      const textarea = screen.getByRole('textbox');
      const box = textarea.closest('div.rounded-3xl') as HTMLElement;
      fireEvent.dragOver(box, { dataTransfer: { types: ['Files'] } });
      expect(screen.getByText('chat.attachment.dropToAttach')).toBeInTheDocument();
    });

    it('routes dropped files through onAttachFiles', () => {
      const onAttachFiles = vi.fn().mockResolvedValue(undefined);
      renderComposer({ onAttachFiles });
      const textarea = screen.getByRole('textbox');
      const box = textarea.closest('div.rounded-3xl') as HTMLElement;
      const file = makeVideoFile();
      fireEvent.drop(box, { dataTransfer: { files: [file], types: ['Files'] } });
      expect(onAttachFiles).toHaveBeenCalledTimes(1);
    });

    it('does not handle drops when attachments are disabled', () => {
      const onAttachFiles = vi.fn().mockResolvedValue(undefined);
      renderComposer({ onAttachFiles, attachmentsEnabled: false });
      const textarea = screen.getByRole('textbox');
      const box = textarea.closest('div.rounded-3xl') as HTMLElement;
      fireEvent.drop(box, { dataTransfer: { files: [makeVideoFile()], types: ['Files'] } });
      expect(onAttachFiles).not.toHaveBeenCalled();
    });

    it('prevents browser file-navigation on drop even when disabled', () => {
      // fireEvent returns false when the event default was prevented.
      renderComposer({ attachmentsEnabled: false });
      const textarea = screen.getByRole('textbox');
      const box = textarea.closest('div.rounded-3xl') as HTMLElement;
      const notPrevented = fireEvent.drop(box, {
        dataTransfer: { files: [makeVideoFile()], types: ['Files'] },
      });
      expect(notPrevented).toBe(false);
    });
  });

  describe('clipboard paste', () => {
    it('attaches image files pasted from the clipboard', () => {
      const onAttachFiles = vi.fn().mockResolvedValue(undefined);
      renderComposer({ onAttachFiles });
      const file = new File([new Uint8Array(4)], 'pasted.png', { type: 'image/png' });
      fireEvent.paste(screen.getByRole('textbox'), {
        clipboardData: { items: [{ kind: 'file', type: 'image/png', getAsFile: () => file }] },
      });
      expect(onAttachFiles).toHaveBeenCalledTimes(1);
      expect(onAttachFiles).toHaveBeenCalledWith([file]);
    });

    it('ignores plain-text paste (no media items)', () => {
      const onAttachFiles = vi.fn().mockResolvedValue(undefined);
      renderComposer({ onAttachFiles });
      fireEvent.paste(screen.getByRole('textbox'), {
        clipboardData: { items: [{ kind: 'string', type: 'text/plain', getAsFile: () => null }] },
      });
      expect(onAttachFiles).not.toHaveBeenCalled();
    });
  });

  describe('render-loop detection guard', () => {
    it('does not warn under normal rendering', () => {
      const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
      renderComposer();
      expect(screen.getByRole('textbox')).toBeInTheDocument();
      // Normal rendering should not trigger the render-loop guard.
      expect(warnSpy).not.toHaveBeenCalledWith(expect.stringContaining('Render-loop detected'));
      warnSpy.mockRestore();
    });

    it('warns when render-loop threshold is exceeded', () => {
      const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
      const textInputRef = createRef<HTMLTextAreaElement | null>();
      const fileInputRef = createRef<HTMLInputElement | null>();
      const isComposingTextRef = { current: false };
      // This test harness simulates a render loop by calling setState
      // from a useEffect, exceeding the guard's 30-render threshold
      // without triggering React's native "too many re-renders" error.
      function LoopHarness() {
        const [count, setCount] = useState(0);
        const [text, setText] = useState('');

        useEffect(() => {
          if (count < 35) {
            setCount(count + 1);
          }
        }, [count, setCount]);

        return (
          <ChatComposer
            inputValue={text}
            setInputValue={v => setText(typeof v === 'function' ? v('') : v)}
            onSend={vi.fn().mockResolvedValue(undefined)}
            textInputRef={textInputRef}
            fileInputRef={fileInputRef}
            composerInteractionBlocked={false}
            isSending={false}
            attachments={[]}
            onAttachFiles={vi.fn().mockResolvedValue(undefined)}
            onRemoveAttachment={vi.fn()}
            attachError={null}
            onSwitchToMicCloud={vi.fn()}
            handleInputKeyDown={vi.fn()}
            inlineCompletionSuffix=""
            isComposingTextRef={isComposingTextRef}
            maxAttachments={5}
            allowedMimeTypes={[]}
            attachmentsEnabled={false}
          />
        );
      }

      render(<LoopHarness />, { wrapper: Runtime });

      const loopCalls = warnSpy.mock.calls.filter(
        args => typeof args[0] === 'string' && args[0].includes('Render-loop detected')
      );
      expect(loopCalls.length).toBeGreaterThan(0);
      expect(loopCalls[0][0]).toContain('ChatComposer');
      warnSpy.mockRestore();
    });

    it('recovers from a temporary loop (counter resets between ticks)', async () => {
      // After a render loop ends, the setTimeout(0) in the guard should reset
      // the counter, so a subsequent normal render doesn't trigger a warning.
      const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
      const textInputRef = createRef<HTMLTextAreaElement | null>();
      const fileInputRef = createRef<HTMLInputElement | null>();
      const isComposingTextRef = { current: false };

      // First, render normally.
      const { rerender } = render(
        <Runtime>
          <ChatComposer
            inputValue=""
            setInputValue={vi.fn()}
            onSend={vi.fn().mockResolvedValue(undefined)}
            textInputRef={textInputRef}
            fileInputRef={fileInputRef}
            composerInteractionBlocked={false}
            isSending={false}
            attachments={[]}
            onAttachFiles={vi.fn().mockResolvedValue(undefined)}
            onRemoveAttachment={vi.fn()}
            attachError={null}
            onSwitchToMicCloud={vi.fn()}
            handleInputKeyDown={vi.fn()}
            inlineCompletionSuffix=""
            isComposingTextRef={isComposingTextRef}
            maxAttachments={5}
            allowedMimeTypes={[]}
            attachmentsEnabled={false}
          />
        </Runtime>
      );

      // Wait for the setTimeout(0) reset to fire.
      await new Promise(r => setTimeout(r, 50));

      // Then render again — should not warn because the counter was reset.
      rerender(
        <Runtime>
          <ChatComposer
            inputValue="hello"
            setInputValue={vi.fn()}
            onSend={vi.fn().mockResolvedValue(undefined)}
            textInputRef={textInputRef}
            fileInputRef={fileInputRef}
            composerInteractionBlocked={false}
            isSending={false}
            attachments={[]}
            onAttachFiles={vi.fn().mockResolvedValue(undefined)}
            onRemoveAttachment={vi.fn()}
            attachError={null}
            onSwitchToMicCloud={vi.fn()}
            handleInputKeyDown={vi.fn()}
            inlineCompletionSuffix=""
            isComposingTextRef={isComposingTextRef}
            maxAttachments={5}
            allowedMimeTypes={[]}
            attachmentsEnabled={false}
          />
        </Runtime>
      );

      const loopCalls = warnSpy.mock.calls.filter(
        args => typeof args[0] === 'string' && args[0].includes('Render-loop detected')
      );
      expect(loopCalls).toHaveLength(0);
      warnSpy.mockRestore();
    });
  });

  describe('follow-up / parallel mode (allowParallelSend during a streaming turn)', () => {
    it('keeps the textarea editable even while an in-flight turn is sending', () => {
      renderComposer({
        allowParallelSend: true,
        composerInteractionBlocked: true,
        isSending: true,
        inputValue: 'a follow-up',
      });
      expect(screen.getByRole('textbox')).not.toBeDisabled();
    });

    it('enables the send button so a follow-up can be queued mid-stream', () => {
      renderComposer({
        allowParallelSend: true,
        composerInteractionBlocked: true,
        isSending: true,
        inputValue: 'a follow-up',
      });
      expect(screen.getByTestId('send-message-button')).not.toBeDisabled();
    });

    it('shows the send arrow (not the in-flight spinner) while queueing', () => {
      const { container } = renderComposer({
        allowParallelSend: true,
        composerInteractionBlocked: true,
        isSending: true,
        inputValue: 'a follow-up',
      });
      expect(container.querySelector('.animate-spin')).toBeNull();
    });

    it('still disables the send button with no typed content mid-stream', () => {
      renderComposer({
        allowParallelSend: true,
        composerInteractionBlocked: true,
        isSending: true,
        inputValue: '',
      });
      expect(screen.getByTestId('send-message-button')).toBeDisabled();
    });

    it('surfaces the follow-up hint as the placeholder', () => {
      renderComposer({ allowParallelSend: true, composerInteractionBlocked: true });
      expect(screen.getByRole('textbox')).toHaveAttribute('placeholder', 'chat.followupHint');
    });
  });

  /**
   * Action-row layout. The reference design puts the attach button and the
   * read-only model chip on the left, and the mic beside the send/stop action
   * on the right. These assert the *grouping and order* of the controls (via
   * DOM containment and document order) rather than any class string, so a
   * restyle cannot break them but a re-ordering will.
   */
  describe('action row layout', () => {
    it('renders the model chip immediately after the attach button', () => {
      renderComposer();
      const attach = screen.getByRole('button', { name: 'composer.attachFile' });
      const chip = screen.getByRole('button', { name: 'composer.modelSelector' });
      expect(chip).toBeInTheDocument();
      // Same group, chip second.
      expect(chip.parentElement).toBe(attach.parentElement);
      expect(attach.compareDocumentPosition(chip) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    });

    it('groups the mic with the send action, not with the attach button', () => {
      renderComposer();
      const attach = screen.getByRole('button', { name: 'composer.attachFile' });
      const mic = screen.getByRole('button', { name: 'composer.voiceMode' });
      const send = screen.getByTestId('send-message-button');
      expect(mic.parentElement).toBe(send.parentElement);
      expect(mic.parentElement).not.toBe(attach.parentElement);
      // Mic precedes send within that group.
      expect(mic.compareDocumentPosition(send) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    });

    it('keeps send reachable and usable with every control present', () => {
      renderComposer({ inputValue: 'hi' });
      expect(screen.getByRole('button', { name: 'composer.attachFile' })).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'composer.modelSelector' })).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'composer.voiceMode' })).toBeInTheDocument();
      const send = screen.getByTestId('send-message-button');
      expect(send).toBeInTheDocument();
      expect(send).not.toBeDisabled();
    });

    it('still shows the model chip when the mic is hidden', () => {
      renderComposer({ micEnabled: false });
      expect(screen.queryByRole('button', { name: 'composer.voiceMode' })).not.toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'composer.modelSelector' })).toBeInTheDocument();
      expect(screen.getByTestId('send-message-button')).toBeInTheDocument();
    });
  });
});
