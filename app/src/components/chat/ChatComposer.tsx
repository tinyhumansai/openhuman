import { useEffect } from 'react';

import type { ChatSendError } from '../../chat/chatSendError';
import type { Attachment } from '../../lib/attachments';
import { useT } from '../../lib/i18n/I18nContext';
import AttachmentPreview from './AttachmentPreview';

/** Max composer height ≈ 8 lines of text-sm + padding. */
const COMPOSER_MAX_HEIGHT = 192;

export interface ChatComposerProps {
  inputValue: string;
  setInputValue: (value: string | ((prev: string) => string)) => void;
  onSend: (text?: string) => Promise<void>;
  textInputRef: React.RefObject<HTMLTextAreaElement | null>;
  fileInputRef: React.RefObject<HTMLInputElement | null>;
  composerInteractionBlocked: boolean;
  isSending: boolean;
  /**
   * When true, the selected thread has an in-flight turn but the composer stays
   * usable: plain Enter / the Send button queue a follow-up (sent after the
   * current turn), and Cmd/Ctrl+Enter forks a parallel branch. Keeps the
   * textarea + send button editable even though `composerInteractionBlocked` is
   * set, and surfaces a follow-up hint instead of showing the in-flight spinner.
   */
  allowParallelSend?: boolean;
  attachments: Attachment[];
  onAttachFiles: (files: FileList | null) => Promise<void>;
  onRemoveAttachment: (id: string) => void;
  attachError: ChatSendError | null;
  onSwitchToMicCloud: () => void;
  handleInputKeyDown: (e: React.KeyboardEvent<HTMLTextAreaElement>) => void;
  inlineCompletionSuffix: string;
  isComposingTextRef: React.MutableRefObject<boolean>;
  maxAttachments: number;
  allowedMimeTypes: readonly string[];
  /**
   * Whether chat multimodal attachments are available. When `false`, the
   * attach button, hidden file input, and preview strip are not rendered.
   */
  attachmentsEnabled: boolean;
  /**
   * Optional nodes stacked above the input box but *outside* its focus
   * highlight — e.g. the queued-follow-ups strip and the thread-goal editor.
   * They render within the overall composer component (so they move with it)
   * yet are not wrapped by the blue focus-within ring/border of the input box.
   * Entries that render `null` contribute nothing.
   */
  headerSlots?: React.ReactNode[];
}

/**
 * Single-row chat composer: [+] textarea [mic] [send]
 * Buttons sit at the bottom-end of the row so they stay anchored when the
 * textarea grows with multiline input.
 */
export default function ChatComposer({
  inputValue,
  setInputValue,
  onSend,
  textInputRef,
  fileInputRef,
  composerInteractionBlocked,
  isSending,
  allowParallelSend = false,
  attachments,
  onAttachFiles,
  onRemoveAttachment,
  attachError: _attachError,
  onSwitchToMicCloud,
  handleInputKeyDown,
  inlineCompletionSuffix,
  isComposingTextRef,
  maxAttachments,
  allowedMimeTypes,
  attachmentsEnabled,
  headerSlots = [],
}: ChatComposerProps) {
  const { t } = useT();

  // While a turn streams (`allowParallelSend`) the composer stays usable for a
  // queued follow-up / parallel branch, so the in-flight `isSending` spinner
  // and lock no longer apply — only real typed content gates the send button.
  const hasContent =
    inputValue.trim().length > 0 || attachments.length > 0 || (isSending && !allowParallelSend);
  // The textarea (and send button) stay editable while a turn streams so the
  // user can queue a follow-up or fork a parallel branch; otherwise an in-flight
  // turn (`composerInteractionBlocked`/`isSending`) locks the composer.
  const composerLocked = !allowParallelSend && (composerInteractionBlocked || isSending);
  const textareaDisabled = composerLocked;
  // Show the working spinner only for a normal in-flight send, not while the
  // composer is intentionally open for follow-up/parallel queueing.
  const showSendingSpinner = isSending && !allowParallelSend;

  // Auto-resize textarea: grow with content, cap at COMPOSER_MAX_HEIGHT, then scroll.
  useEffect(() => {
    const ta = textInputRef.current;
    if (!ta) return;
    ta.style.height = 'auto';
    ta.style.height = `${Math.min(ta.scrollHeight, COMPOSER_MAX_HEIGHT)}px`;
    ta.style.overflowY = ta.scrollHeight > COMPOSER_MAX_HEIGHT ? 'auto' : 'hidden';
  }, [inputValue, textInputRef]);

  return (
    <div className="relative flex flex-col gap-1">
      {/* Header stack (e.g. queued follow-ups, thread-goal editor): rendered
          above the input box and OUTSIDE its blue focus highlight, but still
          within the overall composer component so they move as one unit. */}
      {headerSlots}

      {/* The input box — only this carries the focus-within highlight. */}
      <div className="relative flex flex-col rounded-2xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 transition-all focus-within:border-primary-500/50 focus-within:ring-1 focus-within:ring-primary-500/50">
        {/* Hidden file input for attachment (gated — see attachmentsEnabled). */}
        {attachmentsEnabled && (
          <input
            ref={fileInputRef}
            type="file"
            // No `accept` filter: Chromium 146 / CEF on macOS greys out valid files
            // at the native open panel regardless of the filter shape (MIME, mixed,
            // or extension-only). Selection is gated in `onAttachFiles` →
            // `validateAndReadFile` instead (rejects unsupported types + images on
            // non-vision models). `allowedMimeTypes` is kept for that JS validation.
            accept={allowedMimeTypes.length ? allowedMimeTypes.join(',') : undefined}
            multiple
            className="hidden"
            onChange={e => {
              void onAttachFiles(e.target.files);
              e.target.value = '';
            }}
          />
        )}

        {/* Attachment preview strip */}
        {attachmentsEnabled && attachments.length > 0 && (
          <div className="px-3 pt-2.5">
            <AttachmentPreview
              attachments={attachments}
              onRemove={onRemoveAttachment}
              disabled={composerInteractionBlocked || isSending}
            />
          </div>
        )}

        {/* Single row: [+] textarea [mic] [send] */}
        <div className="flex items-center gap-2 p-3">
          {/* Attach button */}
          {attachmentsEnabled && (
            <button
              type="button"
              data-analytics-id="chat-composer-attach-file"
              aria-label={t('composer.attachFile')}
              title={t('composer.attachFile')}
              onClick={() => fileInputRef.current?.click()}
              disabled={
                composerInteractionBlocked || isSending || attachments.length >= maxAttachments
              }
              className="flex-shrink-0 flex items-center justify-center w-6 h-6 text-stone-400 dark:text-neutral-500 hover:text-stone-600 dark:hover:text-neutral-300 transition-colors disabled:opacity-40 disabled:cursor-not-allowed">
              <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={1.8}
                  d="M12 5v14m-7-7h14"
                />
              </svg>
            </button>
          )}

          {/* Textarea with ghost completion */}
          <div className="relative flex-1 align-middle flex min-w-0">
            <div
              aria-hidden
              className="pointer-events-none absolute inset-0 overflow-hidden whitespace-pre-wrap break-words py-0.5 text-sm leading-5 font-sans">
              <span className="invisible">{inputValue}</span>
              <span className="text-stone-500 dark:text-neutral-400/50">
                {inlineCompletionSuffix}
              </span>
            </div>
            <textarea
              ref={textInputRef}
              value={inputValue}
              onChange={e => setInputValue(e.target.value)}
              onCompositionStart={() => {
                isComposingTextRef.current = true;
              }}
              onCompositionEnd={() => {
                isComposingTextRef.current = false;
              }}
              onKeyDown={handleInputKeyDown}
              placeholder={allowParallelSend ? t('chat.followupHint') : t('chat.typeMessage')}
              rows={1}
              disabled={textareaDisabled}
              className="relative z-10 w-full resize-none border-0 bg-transparent py-0.5 px-0.5 text-sm leading-5 whitespace-pre-wrap break-words font-sans text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 outline-none focus:outline-none focus-visible:outline-none focus:ring-0 focus-visible:ring-0 overflow-hidden disabled:opacity-50 disabled:cursor-not-allowed"
            />
          </div>

          {/* Voice mode */}
          <button
            type="button"
            data-analytics-id="chat-composer-voice-mode"
            aria-label={t('composer.voiceMode')}
            title={t('composer.voiceMode')}
            onClick={onSwitchToMicCloud}
            disabled={composerInteractionBlocked || isSending}
            className="flex-shrink-0 flex items-center justify-center w-6 h-6 text-stone-400 dark:text-neutral-500 hover:text-stone-600 dark:hover:text-neutral-300 transition-colors disabled:opacity-40 disabled:cursor-not-allowed">
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M12 1a3 3 0 00-3 3v8a3 3 0 006 0V4a3 3 0 00-3-3z"
              />
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M19 10v2a7 7 0 01-14 0v-2M12 19v4m-4 0h8"
              />
            </svg>
          </button>

          {/* Send button */}
          <button
            type="button"
            data-analytics-id="chat-composer-send"
            data-testid="send-message-button"
            aria-label={t('chat.send')}
            title={t('chat.send')}
            onClick={() => {
              void onSend();
            }}
            disabled={!hasContent || composerLocked}
            className="flex-shrink-0 flex items-center justify-center w-6 h-6 rounded-full bg-primary-500 hover:bg-primary-600 text-white disabled:opacity-40 disabled:cursor-not-allowed transition-colors">
            {showSendingSpinner ? (
              <svg className="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
                <circle
                  className="opacity-25"
                  cx="12"
                  cy="12"
                  r="10"
                  stroke="currentColor"
                  strokeWidth="4"
                />
                <path
                  className="opacity-75"
                  fill="currentColor"
                  d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
                />
              </svg>
            ) : (
              <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2.5}
                  d="M9 5l7 7-7 7"
                />
              </svg>
            )}
          </button>
        </div>
      </div>
    </div>
  );
}
