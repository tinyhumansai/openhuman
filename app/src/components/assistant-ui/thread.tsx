'use client';

import {
  ComposerAddAttachment,
  ComposerAttachments,
  UserMessageAttachments,
} from '@/components/assistant-ui/attachment';
import { ComposerTriggerPopover } from '@/components/assistant-ui/composer-trigger-popover';
import { File } from '@/components/assistant-ui/file';
import { ThreadFollowupSuggestions } from '@/components/assistant-ui/follow-up-suggestions';
import { Image } from '@/components/assistant-ui/image';
import { cn } from '@/components/assistant-ui/lib/utils';
import { MarkdownText } from '@/components/assistant-ui/markdown-text';
import {
  Reasoning,
  ReasoningContent,
  ReasoningRoot,
  ReasoningText,
  ReasoningTrigger,
} from '@/components/assistant-ui/reasoning';
import { ToolFallback } from '@/components/assistant-ui/tool-fallback';
import {
  ToolGroupContent,
  ToolGroupRoot,
  ToolGroupTrigger,
} from '@/components/assistant-ui/tool-group';
import { TooltipIconButton } from '@/components/assistant-ui/tooltip-icon-button';
import { Button } from '@/components/assistant-ui/ui/button';
import { Skeleton } from '@/components/assistant-ui/ui/skeleton';
import ModelQualityPill from '@/components/chat/ModelQualityPill';
import {
  ActionBarMorePrimitive,
  ActionBarPrimitive,
  type AssistantState,
  AuiIf,
  BranchPickerPrimitive,
  ComposerPrimitive,
  ErrorPrimitive,
  type FileMessagePartComponent,
  groupPartByType,
  type ImageMessagePartComponent,
  MessagePrimitive,
  SuggestionPrimitive,
  ThreadPrimitive,
  type ToolCallMessagePartComponent,
  type Unstable_SlashCommand,
  unstable_useSlashCommandAdapter,
  useAui,
  useAuiState,
} from '@assistant-ui/react';
import { LexicalComposerInput } from '@assistant-ui/react-lexical';
import {
  ArrowDownIcon,
  ArrowUpIcon,
  CheckIcon,
  ChevronLeftIcon,
  ChevronRightIcon,
  CopyIcon,
  DownloadIcon,
  MicIcon,
  MoreHorizontalIcon,
  PencilIcon,
  RefreshCwIcon,
  SlashIcon,
  SquareIcon,
} from 'lucide-react';
import {
  type ComponentType,
  createContext,
  type FC,
  type PropsWithChildren,
  useContext,
  useEffect,
  useRef,
} from 'react';

export type ThreadGroupPart = MessagePrimitive.GroupedParts.GroupPart;

/**
 * Optional component overrides for the thread. `AssistantMessage` and
 * `Welcome` replace whole sections; the remaining slots override how the
 * assistant message renders tool calls and part groups. Tool UIs registered
 * by name (toolkit `render`, `useAssistantDataUI`) take precedence over
 * `ToolFallback`.
 */
export type ThreadComponents = {
  AssistantMessage?: ComponentType | undefined;
  Welcome?: ComponentType | undefined;
  ToolFallback?: ToolCallMessagePartComponent | undefined;
  ToolGroup?: ComponentType<PropsWithChildren<{ group: ThreadGroupPart }>> | undefined;
  ReasoningGroup?: ComponentType<PropsWithChildren<{ group: ThreadGroupPart }>> | undefined;
  /**
   * Extra controls in the composer's action row, to the right of the model
   * selector. A seam rather than a fixed set because what belongs there is
   * host-specific — OpenHuman puts the context-window meter and the thread
   * goal here — and hard-coding either would make this component unusable by
   * anything else.
   */
  ComposerExtras?: ComponentType | undefined;
  /** Full-width host content immediately above the composer shell. */
  ComposerHeader?: ComponentType | undefined;
  /** Host-owned attachment previews rendered above the editor. */
  ComposerAttachments?: ComponentType | undefined;
  /** Host-owned attachment picker rendered in the action row. */
  ComposerAddAttachment?: ComponentType | undefined;
  /** Enables sending when the host has attachments but the editor is empty. */
  hasComposerAttachments?: boolean | undefined;
  /** Sends an attachment-only message through the host's normal send path. */
  onComposerAttachmentSend?: (() => void) | undefined;
  /**
   * Host-owned control for the composer's primary slot while there is nothing
   * to send — the slot ChatGPT gives its voice mode. Send takes the slot back
   * on the first character or attachment, and a running turn always shows
   * Cancel. A component rather than a callback for the same reason
   * `ComposerAddAttachment` is one: what belongs there is the host's own
   * branding and behaviour, and this file should not learn about either.
   */
  ComposerIdleAction?: ComponentType | undefined;
  /** Switches the host chat surface into its microphone-first composer. */
  onSwitchToMicCloud?: (() => void) | undefined;
};

export type ThreadProps = {
  components?: ThreadComponents | undefined;
  /** Host-owned model route used for real sends. */
  model?: string | null | undefined;
  /** Updates the host's composer route and selected model metadata. */
  onModelChange?: ((value: string | null, contextWindow?: number | null) => void) | undefined;
  /** Host transport error shown in place of an empty welcome state. */
  loadError?: string | null | undefined;
  /** Host-specific Escape behavior (for example cancel + restore prompt). */
  onEscape?: (() => void) | undefined;
  /**
   * Commands offered when the composer input starts with `/`. Supplied by the
   * host because a command's `execute` is host behaviour (`/clear` has to
   * reach a runtime this component does not own).
   */
  slashCommands?: readonly Unstable_SlashCommand[] | undefined;
};

const EMPTY_COMPONENTS: ThreadComponents = {};

const ThreadComponentsContext = createContext<ThreadComponents>(EMPTY_COMPONENTS);

const NO_SLASH_COMMANDS: readonly Unstable_SlashCommand[] = [];
const SlashCommandsContext = createContext<readonly Unstable_SlashCommand[]>(NO_SLASH_COMMANDS);

// Startup exposes a loading placeholder thread; treat it as a new chat so
// the composer mounts centered. Loads after startup keep the docked layout.
const isNewChatView = (s: AssistantState) =>
  s.thread.messages.length === 0 && (!s.thread.isLoading || s.threads.isLoading);

// A switched thread that is still fetching its history: skeleton, not welcome.
const isHistoryLoadingView = (s: AssistantState) =>
  s.thread.messages.length === 0 &&
  s.thread.isLoading &&
  !s.thread.isDisabled &&
  !s.threads.isLoading;

const ThreadHistorySkeleton: FC = () => (
  <div
    data-slot="aui_thread-history-skeleton"
    role="status"
    className="animate-in fade-in fill-mode-both flex flex-col gap-y-6 [animation-delay:150ms] [animation-duration:200ms]">
    <span className="sr-only">Loading conversation</span>
    <Skeleton className="ml-auto h-9 w-2/5 rounded-xl motion-reduce:animate-none" />
    <div className="flex flex-col gap-y-2">
      <Skeleton className="h-4 w-11/12 motion-reduce:animate-none" />
      <Skeleton className="h-4 w-4/5 motion-reduce:animate-none" />
      <Skeleton className="h-4 w-3/5 motion-reduce:animate-none" />
    </div>
    <Skeleton className="ml-auto h-9 w-1/3 rounded-xl motion-reduce:animate-none" />
    <div className="flex flex-col gap-y-2">
      <Skeleton className="h-4 w-10/12 motion-reduce:animate-none" />
      <Skeleton className="h-4 w-2/3 motion-reduce:animate-none" />
    </div>
  </div>
);

export const Thread: FC<ThreadProps> = ({
  components = EMPTY_COMPONENTS,
  model = 'hint:chat',
  onModelChange,
  loadError = null,
  onEscape,
  slashCommands = NO_SLASH_COMMANDS,
}) => {
  const isEmpty = useAuiState(isNewChatView);

  return (
    <ThreadComponentsContext.Provider value={components}>
      <SlashCommandsContext.Provider value={slashCommands}>
        <ThreadRoot
          isEmpty={isEmpty}
          model={model}
          onModelChange={onModelChange}
          loadError={loadError}
          onEscape={onEscape}
        />
      </SlashCommandsContext.Provider>
    </ThreadComponentsContext.Provider>
  );
};

const ThreadRoot: FC<{
  isEmpty: boolean;
  model: string | null;
  onModelChange?: (value: string | null, contextWindow?: number | null) => void;
  loadError: string | null;
  onEscape?: () => void;
}> = ({ isEmpty, model, onModelChange, loadError, onEscape }) => {
  const { Welcome = ThreadWelcome } = useContext(ThreadComponentsContext);

  return (
    <ThreadPrimitive.Root
      className="aui-root aui-thread-root bg-background @container flex h-full flex-col"
      style={{
        ['--thread-max-width' as string]: '44rem',
        ['--composer-bg' as string]: 'var(--color-card)',
        ['--composer-radius' as string]: '1.5rem',
        ['--composer-padding' as string]: '8px',
      }}>
      <ThreadPrimitive.Viewport
        turnAnchor="top"
        data-slot="aui_thread-viewport"
        className="relative flex flex-1 flex-col overflow-x-auto overflow-y-scroll scroll-smooth">
        <div
          className={cn(
            'mx-auto flex w-full max-w-(--thread-max-width) flex-1 flex-col px-4 pt-4',
            isEmpty && 'justify-center'
          )}>
          {loadError ? (
            <div className="flex flex-1 flex-col items-center justify-center gap-2 text-center">
              <p className="text-sm font-medium text-destructive">Failed to load messages</p>
              <p className="text-muted-foreground max-w-md text-xs">{loadError}</p>
            </div>
          ) : (
            <>
              <AuiIf condition={isNewChatView}>
                <Welcome />
              </AuiIf>
              <AuiIf condition={isHistoryLoadingView}>
                <ThreadHistorySkeleton />
              </AuiIf>
            </>
          )}

          <div data-slot="aui_message-group" className="mb-14 flex flex-col gap-y-6 empty:hidden">
            <ThreadPrimitive.Messages>{() => <ThreadMessage />}</ThreadPrimitive.Messages>
          </div>

          <ThreadPrimitive.ViewportFooter
            className={cn(
              'aui-thread-viewport-footer bg-background flex flex-col gap-4 overflow-visible pb-4 md:pb-6',
              !isEmpty && 'sticky bottom-0 mt-auto rounded-t-(--composer-radius)'
            )}>
            <ThreadScrollToBottom />
            <ThreadFollowupSuggestions />
            <Composer model={model} onModelChange={onModelChange} onEscape={onEscape} />
            <AuiIf condition={s => isNewChatView(s) && s.composer.isEmpty}>
              <ThreadSuggestions />
            </AuiIf>
          </ThreadPrimitive.ViewportFooter>
        </div>
      </ThreadPrimitive.Viewport>
    </ThreadPrimitive.Root>
  );
};

const ThreadMessage: FC = () => {
  const { AssistantMessage: AssistantMessageComponent = AssistantMessage } =
    useContext(ThreadComponentsContext);
  const role = useAuiState(s => s.message.role);
  const isEditing = useAuiState(s => s.message.composer.isEditing);

  if (isEditing) return <EditComposer />;
  if (role === 'user') return <UserMessage />;
  return <AssistantMessageComponent />;
};

const ThreadScrollToBottom: FC = () => {
  return (
    <ThreadPrimitive.ScrollToBottom asChild>
      <TooltipIconButton
        tooltip="Scroll to bottom"
        variant="outline"
        className="aui-thread-scroll-to-bottom dark:border-border dark:bg-background dark:hover:bg-accent absolute -top-12 z-10 self-center rounded-full p-4 disabled:invisible">
        <ArrowDownIcon />
      </TooltipIconButton>
    </ThreadPrimitive.ScrollToBottom>
  );
};

const ThreadWelcome: FC = () => {
  return (
    <div className="aui-thread-welcome-root mb-6 flex flex-col items-center px-4 text-center">
      <h1 className="aui-thread-welcome-message-inner fade-in slide-in-from-bottom-1 animate-in fill-mode-both text-2xl font-medium tracking-tight duration-200">
        How can I help you today?
      </h1>
    </div>
  );
};

const ThreadSuggestions: FC = () => {
  return (
    <div className="aui-thread-welcome-suggestions flex w-full flex-wrap items-center justify-center gap-2 px-4">
      <ThreadPrimitive.Suggestions>{() => <ThreadSuggestionItem />}</ThreadPrimitive.Suggestions>
    </div>
  );
};

const ThreadSuggestionItem: FC = () => {
  return (
    <div className="aui-thread-welcome-suggestion-display fade-in slide-in-from-bottom-2 animate-in fill-mode-both duration-200">
      <SuggestionPrimitive.Trigger send asChild>
        <Button
          variant="ghost"
          className="aui-thread-welcome-suggestion text-foreground hover:bg-muted border-border/60 h-auto gap-1.5 rounded-full border px-3.5 py-1.5 text-sm font-normal whitespace-nowrap transition-colors">
          <SuggestionPrimitive.Title className="aui-thread-welcome-suggestion-text-1" />
          <SuggestionPrimitive.Description className="aui-thread-welcome-suggestion-text-2 empty:hidden" />
        </Button>
      </SuggestionPrimitive.Trigger>
    </div>
  );
};

const Composer: FC<{
  model: string | null;
  onModelChange?: (value: string | null, contextWindow?: number | null) => void;
  onEscape?: () => void;
}> = ({ model, onModelChange, onEscape }) => {
  const aui = useAui();
  const commands = useContext(SlashCommandsContext);
  const slash = unstable_useSlashCommandAdapter({ commands, fallbackIcon: SlashIcon });
  const inputWrapperRef = useRef<HTMLDivElement>(null);
  // IME composition writes pre-edit text into the contenteditable's
  // `textContent` before Lexical commits it to editor state. Pushing that
  // intermediate state through `aui.composer.setText` makes the package's
  // SyncPlugin rebuild the editor mid-composition, which cancels the
  // composition and commits each pre-edit stage as literal text
  // ("n ni nihao 你好"). The gate keeps the DOM→store bridge below closed
  // while composing; see the comment on the bridge for why the bridge itself
  // must stay.
  const isComposingTextRef = useRef(false);
  const { ComposerHeader, ComposerAttachments: HostComposerAttachments } =
    useContext(ThreadComponentsContext);
  useEffect(() => {
    const textbox = inputWrapperRef.current?.querySelector<HTMLElement>('[contenteditable="true"]');
    textbox?.setAttribute('aria-label', 'Message input');
    // The rich Lexical surface deliberately is not a native textarea, so give
    // it an explicit stable hook for browser tests and assistive tooling. The
    // old chat composer exposed a textarea with a placeholder; consumers must
    // not have to depend on Lexical's internal DOM shape to find the primary
    // message input.
    textbox?.setAttribute('data-testid', 'chat-message-input');
    return () => {
      textbox?.removeAttribute('aria-label');
      textbox?.removeAttribute('data-testid');
    };
  }, []);

  return (
    <ComposerPrimitive.Unstable_TriggerPopoverRoot>
      <ComposerPrimitive.Root
        className="aui-composer-root relative flex w-full flex-col"
        data-walkthrough="chat-agent-panel">
        {ComposerHeader ? <ComposerHeader /> : null}
        <ComposerPrimitive.AttachmentDropzone asChild>
          <div
            data-slot="aui_composer-shell"
            className="border-line focus-within:border-line-strong data-[dragging=true]:border-ring flex w-full cursor-text flex-col gap-2 rounded-(--composer-radius) border bg-(--composer-bg) p-(--composer-padding) transition-[border-color] data-[dragging=true]:border-dashed data-[dragging=true]:bg-[color-mix(in_oklab,var(--color-accent)_50%,var(--color-background))]">
            {HostComposerAttachments ? <HostComposerAttachments /> : <ComposerAttachments />}
            {/*
             * Lexical rather than the plain `ComposerPrimitive.Input` textarea,
             * because `/` commands need a rich input: the trigger popover has to
             * anchor to the caret and the accepted command has to become a chip
             * rather than literal text the model would read. `commands` is empty
             * unless the host supplies some, and with none the popover never
             * opens, so a host that wants a plain box still gets one.
             *
             * The input-level DOM→store bridge below must stay, gated on
             * composition: Lexical's own editor-state commits do not reach the
             * composer store in environments that bypass its input pipeline —
             * jsdom tests drive the editable by setting `textContent` and
             * firing a synthetic `input` event (see `setComposerText` in
             * Conversations.render.test.tsx), so SyncPlugin never fires there
             * and this bridge is the only path. During a real IME composition
             * the same `textContent` read returns pre-edit text that Lexical
             * has NOT committed, and pushing it would make SyncPlugin rebuild
             * the editor mid-composition — hence the composition gate.
             */}
            <LexicalComposerInput
              ref={inputWrapperRef}
              placeholder="Send a message..."
              onInputCapture={event => {
                if (isComposingTextRef.current) return;
                const native = event.nativeEvent;
                if ('isComposing' in native && native.isComposing) return;
                const target = event.target;
                if (target instanceof HTMLElement) {
                  const text = target.textContent ?? '';
                  globalThis.queueMicrotask(() => aui.composer.setText(text));
                }
              }}
              onCompositionStartCapture={() => {
                isComposingTextRef.current = true;
              }}
              onCompositionEndCapture={event => {
                isComposingTextRef.current = false;
                // Chrome finalizes the composed DOM before compositionend
                // fires; WebKit only after the event — and Lexical re-owns
                // (and may clear) the DOM while handling that same event, so
                // neither a synchronous read nor a deferred one is right
                // everywhere. Snapshot the text at capture time, then prefer
                // the finalized DOM one macrotask later, falling back to the
                // snapshot when Lexical has since re-owned the DOM (the jsdom
                // bridge path, where the editor state never saw the text).
                const target = event.target;
                if (!(target instanceof HTMLElement)) return;
                const textAtCompositionEnd = target.textContent ?? '';
                globalThis.setTimeout(() => {
                  const finalized = target.textContent ?? '';
                  aui.composer.setText(finalized || textAtCompositionEnd);
                }, 0);
              }}
              onKeyDownCapture={event => {
                if (event.key === 'Escape' && onEscape) {
                  event.preventDefault();
                  event.stopPropagation();
                  onEscape();
                  return;
                }
                const native = event.nativeEvent;
                if (
                  native.isComposing ||
                  native.keyCode === 229 ||
                  ('which' in native && native.which === 229)
                ) {
                  event.preventDefault();
                  event.stopPropagation();
                }
              }}
              className="aui-composer-input caret-primary [&_.aui-lexical-placeholder]:text-muted-foreground/60 relative max-h-48 min-h-10 w-full resize-none bg-transparent px-2.5 py-1 text-base leading-6 outline-none [&_.aui-lexical-input]:min-h-lh [&_.aui-lexical-input]:outline-none [&_.aui-lexical-placeholder]:pointer-events-none [&_.aui-lexical-placeholder]:absolute [&_.aui-lexical-placeholder]:top-0 [&_.aui-lexical-placeholder]:right-0 [&_.aui-lexical-placeholder]:left-0 [&_.aui-lexical-placeholder]:truncate [&_.aui-lexical-placeholder]:px-2.5 [&_.aui-lexical-placeholder]:py-1"
              aria-label="Message input"
            />
            <ComposerAction model={model} onModelChange={onModelChange} />
          </div>
        </ComposerPrimitive.AttachmentDropzone>

        {commands.length > 0 && (
          <ComposerTriggerPopover char="/" {...slash} emptyItemsLabel="No matching commands" />
        )}
      </ComposerPrimitive.Root>
    </ComposerPrimitive.Unstable_TriggerPopoverRoot>
  );
};

const ComposerExtrasSlot: FC = () => {
  const { ComposerExtras } = useContext(ThreadComponentsContext);
  return ComposerExtras ? <ComposerExtras /> : null;
};

const ComposerAction: FC<{
  model: string | null;
  onModelChange?: (value: string | null, contextWindow?: number | null) => void;
}> = ({ model, onModelChange }) => {
  const aui = useAui();
  const composerText = useAuiState(state => state.composer.text);
  const {
    ComposerAddAttachment: HostComposerAddAttachment,
    hasComposerAttachments,
    onComposerAttachmentSend,
    ComposerIdleAction,
    onSwitchToMicCloud,
  } = useContext(ThreadComponentsContext);
  const isRunning = useAuiState(state => state.thread.isRunning);
  // Nothing to send: the primary slot goes to the host's idle control instead
  // of a Send button that would refuse the click anyway. Guarded on
  // `isRunning` by the surrounding `AuiIf`, so a streaming turn still shows
  // Cancel.
  const showIdleAction =
    !!ComposerIdleAction && composerText.trim().length === 0 && !hasComposerAttachments;
  return (
    <div className="aui-composer-action-wrapper relative flex items-center justify-between">
      <div className="flex min-w-0 items-center gap-1">
        {HostComposerAddAttachment ? <HostComposerAddAttachment /> : <ComposerAddAttachment />}
        <ModelQualityPill value={model} onValueChange={onModelChange} />
        <ComposerExtrasSlot />
      </div>
      <div className="flex items-center gap-1.5">
        {onSwitchToMicCloud && (
          <TooltipIconButton
            tooltip="Voice mode"
            side="bottom"
            type="button"
            variant="ghost"
            size="icon"
            className="aui-composer-voice-mode text-muted-foreground hover:text-foreground size-7 rounded-full"
            aria-label="Voice mode"
            disabled={isRunning}
            onClick={onSwitchToMicCloud}>
            <MicIcon className="size-4" />
          </TooltipIconButton>
        )}
        <AuiIf condition={s => s.thread.capabilities.dictation}>
          <AuiIf condition={s => s.composer.dictation == null}>
            <ComposerPrimitive.Dictate asChild>
              <TooltipIconButton
                tooltip="Voice input"
                side="bottom"
                type="button"
                variant="ghost"
                size="icon"
                className="aui-composer-dictate text-muted-foreground hover:text-foreground size-7 rounded-full"
                aria-label="Start voice input">
                <MicIcon className="aui-composer-dictate-icon size-4" />
              </TooltipIconButton>
            </ComposerPrimitive.Dictate>
          </AuiIf>
          <AuiIf condition={s => s.composer.dictation != null}>
            <ComposerPrimitive.StopDictation asChild>
              <TooltipIconButton
                tooltip="Stop dictation"
                side="bottom"
                type="button"
                variant="ghost"
                size="icon"
                className="aui-composer-stop-dictation text-destructive size-7 rounded-full"
                aria-label="Stop voice input">
                <SquareIcon className="aui-composer-stop-dictation-icon size-3.5 animate-pulse fill-current" />
              </TooltipIconButton>
            </ComposerPrimitive.StopDictation>
          </AuiIf>
        </AuiIf>
        <AuiIf condition={s => !s.thread.isRunning}>
          {showIdleAction ? (
            <ComposerIdleAction />
          ) : hasComposerAttachments && composerText.trim().length === 0 ? (
            <TooltipIconButton
              tooltip="Send message"
              side="bottom"
              type="button"
              variant="default"
              size="icon"
              className="aui-composer-send size-7 rounded-full"
              data-testid="send-message-button"
              aria-label="Send message"
              onClick={() => {
                onComposerAttachmentSend?.();
                aui.composer.setText('');
              }}>
              <ArrowUpIcon className="aui-composer-send-icon size-4" />
            </TooltipIconButton>
          ) : (
            <ComposerPrimitive.Send asChild>
              <TooltipIconButton
                tooltip="Send message"
                side="bottom"
                type="button"
                variant="default"
                size="icon"
                className="aui-composer-send size-7 rounded-full"
                data-testid="send-message-button"
                aria-label="Send message">
                <ArrowUpIcon className="aui-composer-send-icon size-4" />
              </TooltipIconButton>
            </ComposerPrimitive.Send>
          )}
        </AuiIf>
        <AuiIf condition={s => s.thread.isRunning}>
          <ComposerPrimitive.Cancel asChild>
            <Button
              type="button"
              variant="default"
              size="icon"
              className="aui-composer-cancel size-7 rounded-full"
              data-testid="stop-generation-button"
              aria-label="Stop generating">
              <SquareIcon className="aui-composer-cancel-icon size-3.5 fill-current" />
            </Button>
          </ComposerPrimitive.Cancel>
        </AuiIf>
      </div>
    </div>
  );
};

const MessageError: FC = () => {
  return (
    <MessagePrimitive.Error>
      <ErrorPrimitive.Root className="aui-message-error-root border-destructive bg-destructive/10 text-destructive dark:bg-destructive/5 mt-2 rounded-md border p-3 text-sm dark:text-red-200">
        <ErrorPrimitive.Message className="aui-message-error-message line-clamp-2" />
      </ErrorPrimitive.Root>
    </MessagePrimitive.Error>
  );
};

const AssistantMessage: FC = () => {
  const {
    ToolFallback: ToolFallbackComponent = ToolFallback,
    ToolGroup,
    ReasoningGroup,
  } = useContext(ThreadComponentsContext);

  const ACTION_BAR_PT = 'pt-1.5';
  // Keep the action bar inside the contained root's paint box, then cancel its reserved space in flow.
  const ACTION_BAR_HEIGHT = `min-h-7.5 ${ACTION_BAR_PT}`;

  return (
    <MessagePrimitive.Root
      data-slot="aui_assistant-message-root"
      data-role="assistant"
      data-testid="agent-message"
      className="fade-in slide-in-from-bottom-1 animate-in relative -mb-7.5 pb-7.5 duration-150 [contain-intrinsic-size:auto_200px] [content-visibility:auto]">
      {/*
       * One vertical rhythm for the whole message, rather than each part
       * bringing its own margin. Measured before this change the gaps ran
       * 16 / 0 / 0 / 16 / 0 / 0 px — `reasoning-root` carries `mb-4` and
       * nothing else carried anything, so a reasoning block sat apart while a
       * tool group and the prose beneath it touched. `[&>*+*]:mt-3` spaces
       * adjacent blocks evenly and the `mb-0` override neutralises the one
       * component with an opinion. The chain-of-thought wrapper below gets the
       * same pair, because reasoning and tool groups are siblings *inside* it
       * rather than of it, so spacing only the outer level misses them.
       */}
      <div
        data-slot="aui_assistant-message-content"
        className="text-foreground [&>*+*]:mt-3 [&_[data-slot=reasoning-root]]:mb-0 px-2 leading-relaxed wrap-break-word">
        <MessagePrimitive.GroupedParts
          groupBy={groupPartByType({
            reasoning: ['group-chainOfThought', 'group-reasoning'],
            'tool-call': ['group-chainOfThought', 'group-tool'],
            'standalone-tool-call': [],
          })}>
          {({ part, children }) => {
            switch (part.type) {
              case 'group-chainOfThought':
                return (
                  <div data-slot="aui_chain-of-thought" className="[&>*+*]:mt-3">
                    {children}
                  </div>
                );
              case 'group-tool':
                if (ToolGroup) {
                  return <ToolGroup group={part}>{children}</ToolGroup>;
                }
                return (
                  <ToolGroupRoot variant="ghost">
                    <ToolGroupTrigger
                      count={part.indices.length}
                      active={part.status.type === 'running'}
                    />
                    <ToolGroupContent>{children}</ToolGroupContent>
                  </ToolGroupRoot>
                );
              case 'group-reasoning': {
                if (ReasoningGroup) {
                  return <ReasoningGroup group={part}>{children}</ReasoningGroup>;
                }
                const running = part.status.type === 'running';
                return (
                  <ReasoningRoot streaming={running}>
                    <ReasoningTrigger active={running} />
                    <ReasoningContent aria-busy={running}>
                      <ReasoningText>{children}</ReasoningText>
                    </ReasoningContent>
                  </ReasoningRoot>
                );
              }
              case 'text':
                return <MarkdownText />;
              case 'reasoning':
                return <Reasoning {...part} />;
              case 'tool-call':
                return part.toolUI ?? <ToolFallbackComponent {...part} />;
              case 'data':
                return part.dataRendererUI;
              case 'file':
                return (
                  <div data-slot="aui_assistant-message-file" className="py-1">
                    <File {...part} />
                  </div>
                );
              case 'image':
                return (
                  <div data-slot="aui_assistant-message-image" className="py-1">
                    <Image {...part} />
                  </div>
                );
              case 'indicator':
                return (
                  <span
                    data-slot="aui_assistant-message-indicator"
                    className="animate-pulse font-sans"
                    aria-label="Assistant is working">
                    {'●'}
                  </span>
                );
              default:
                return null;
            }
          }}
        </MessagePrimitive.GroupedParts>
        <MessageError />
      </div>

      <div
        data-slot="aui_assistant-message-footer"
        className={cn('ms-2 flex items-center', ACTION_BAR_HEIGHT)}>
        <AuiIf
          condition={s =>
            s.message.status?.type === 'incomplete' && s.message.status.reason === 'cancelled'
          }>
          <span data-testid="stopped-marker" className="text-muted-foreground text-xs">
            Stopped
          </span>
        </AuiIf>
        <BranchPicker />
        <AssistantActionBar />
      </div>
    </MessagePrimitive.Root>
  );
};

const AssistantActionBar: FC = () => {
  return (
    <ActionBarPrimitive.Root
      hideWhenRunning
      autohide="not-last"
      className="aui-assistant-action-bar-root text-muted-foreground animate-in fade-in col-start-3 row-start-2 -ms-1 flex gap-1 duration-200">
      <ActionBarPrimitive.Copy asChild>
        <TooltipIconButton tooltip="Copy">
          <AuiIf condition={s => s.message.isCopied}>
            <CheckIcon className="animate-in zoom-in-50 fade-in duration-200 ease-out" />
          </AuiIf>
          <AuiIf condition={s => !s.message.isCopied}>
            <CopyIcon className="animate-in zoom-in-75 fade-in duration-150" />
          </AuiIf>
        </TooltipIconButton>
      </ActionBarPrimitive.Copy>
      <ActionBarPrimitive.Reload asChild>
        <TooltipIconButton tooltip="Refresh">
          <RefreshCwIcon />
        </TooltipIconButton>
      </ActionBarPrimitive.Reload>
      <ActionBarMorePrimitive.Root>
        <ActionBarMorePrimitive.Trigger asChild>
          <TooltipIconButton tooltip="More" className="data-[state=open]:bg-accent">
            <MoreHorizontalIcon />
          </TooltipIconButton>
        </ActionBarMorePrimitive.Trigger>
        <ActionBarMorePrimitive.Content
          side="bottom"
          align="start"
          sideOffset={6}
          className="aui-action-bar-more-content bg-popover text-popover-foreground data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95 data-[state=open]:animate-in data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-95 data-[state=closed]:animate-out data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2 z-50 min-w-32 overflow-hidden rounded-xl border p-1.5">
          <ActionBarPrimitive.ExportMarkdown asChild>
            <ActionBarMorePrimitive.Item className="aui-action-bar-more-item hover:bg-accent hover:text-accent-foreground focus:bg-accent focus:text-accent-foreground flex cursor-pointer items-center gap-2 rounded-lg px-2.5 py-1.5 text-sm outline-hidden select-none">
              <DownloadIcon className="size-4" />
              Export as Markdown
            </ActionBarMorePrimitive.Item>
          </ActionBarPrimitive.ExportMarkdown>
        </ActionBarMorePrimitive.Content>
      </ActionBarMorePrimitive.Root>
    </ActionBarPrimitive.Root>
  );
};

const UserFilePart: FileMessagePartComponent = part => (
  <div data-slot="aui_user-message-file" className="py-1">
    <File {...part} />
  </div>
);

const UserImagePart: ImageMessagePartComponent = part => (
  <div data-slot="aui_user-message-image" className="py-1">
    <Image {...part} />
  </div>
);

const UserMessage: FC = () => {
  return (
    <MessagePrimitive.Root
      data-slot="aui_user-message-root"
      className="fade-in slide-in-from-bottom-1 animate-in grid auto-rows-auto grid-cols-[minmax(72px,1fr)_auto] content-start gap-y-2 px-2 duration-150 [contain-intrinsic-size:auto_200px] [content-visibility:auto] [&:where(>*)]:col-start-2"
      data-role="user">
      <UserMessageAttachments />

      <div className="aui-user-message-content-wrapper relative col-start-2 min-w-0">
        <div className="aui-user-message-content peer bg-muted text-foreground rounded-xl px-4 py-2 wrap-break-word empty:hidden">
          <MessagePrimitive.Parts components={{ File: UserFilePart, Image: UserImagePart }} />
        </div>
        <div className="aui-user-action-bar-wrapper absolute inset-s-0 top-1/2 -translate-x-full -translate-y-1/2 pe-2 peer-empty:hidden rtl:translate-x-full">
          <UserActionBar />
        </div>
      </div>

      <BranchPicker
        data-slot="aui_user-branch-picker"
        className="col-span-full col-start-1 row-start-3 -me-1 justify-end"
      />
    </MessagePrimitive.Root>
  );
};

const UserActionBar: FC = () => {
  return (
    <ActionBarPrimitive.Root
      hideWhenRunning
      autohide="not-last"
      className="aui-user-action-bar-root flex flex-col items-end">
      <ActionBarPrimitive.Copy asChild>
        <TooltipIconButton tooltip="Copy response" title="Copy response">
          <CopyIcon />
        </TooltipIconButton>
      </ActionBarPrimitive.Copy>
      <ActionBarPrimitive.Edit asChild>
        <TooltipIconButton tooltip="Edit" className="aui-user-action-edit">
          <PencilIcon />
        </TooltipIconButton>
      </ActionBarPrimitive.Edit>
    </ActionBarPrimitive.Root>
  );
};

const EditComposer: FC = () => {
  return (
    <MessagePrimitive.Root
      data-slot="aui_edit-composer-wrapper"
      className="flex flex-col px-2 [contain-intrinsic-size:auto_200px] [content-visibility:auto]">
      <ComposerPrimitive.Root className="aui-edit-composer-root border-border/60 dark:border-muted-foreground/15 ms-auto flex w-full max-w-[85%] cursor-text flex-col rounded-(--composer-radius) border bg-(--composer-bg)">
        <ComposerPrimitive.Input
          className="aui-edit-composer-input text-foreground min-h-14 w-full resize-none bg-transparent px-4 pt-3 pb-1 text-base outline-hidden"
          autoFocus
        />
        <div className="aui-edit-composer-footer mx-2.5 mb-2.5 flex items-center gap-1.5 self-end">
          <ComposerPrimitive.Cancel asChild>
            <Button variant="ghost" size="sm" className="h-8 rounded-full px-3.5">
              Cancel
            </Button>
          </ComposerPrimitive.Cancel>
          <ComposerPrimitive.Send asChild>
            <Button size="sm" className="h-8 rounded-full px-3.5">
              Update
            </Button>
          </ComposerPrimitive.Send>
        </div>
      </ComposerPrimitive.Root>
    </MessagePrimitive.Root>
  );
};

const BranchPicker: FC<BranchPickerPrimitive.Root.Props> = ({ className, ...rest }) => {
  return (
    <BranchPickerPrimitive.Root
      hideWhenSingleBranch
      className={cn(
        'aui-branch-picker-root text-muted-foreground -ms-2 me-2 inline-flex items-center text-xs',
        className
      )}
      {...rest}>
      <BranchPickerPrimitive.Previous asChild>
        <TooltipIconButton tooltip="Previous">
          <ChevronLeftIcon />
        </TooltipIconButton>
      </BranchPickerPrimitive.Previous>
      <span className="aui-branch-picker-state font-medium">
        <BranchPickerPrimitive.Number /> / <BranchPickerPrimitive.Count />
      </span>
      <BranchPickerPrimitive.Next asChild>
        <TooltipIconButton tooltip="Next">
          <ChevronRightIcon />
        </TooltipIconButton>
      </BranchPickerPrimitive.Next>
    </BranchPickerPrimitive.Root>
  );
};
