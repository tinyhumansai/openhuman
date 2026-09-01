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
import { useAuiEditCapabilities } from '@/features/conversations/components/aui/auiThreadState';
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
            // Keyed to `content-faint` rather than `line`/`line-strong`, which
            // sat too close to the composer's own surface to read as an edge at
            // all; `content-faint` is a real step along the grey ramp in both
            // themes and the alpha then pulls it back.
            //
            // The border is deliberately fainter than the content card's edge
            // (0.65 in `index.css`) because it is not carrying the definition
            // alone: `shadow-soft` lifts the composer off the transcript, and a
            // lifted surface needs less outline than a flat one to read as
            // separate. Border and shadow together at low strength read calmer
            // than either at full — a hard 0.65 line under a shadow reads as
            // two competing edges.
            //
            // Two roles, kept apart: the SHADOW is constant and the BORDER is
            // what moves.
            //
            // The shadow is an explicit near-black pair rather than
            // `shadow-soft`/`shadow-medium`. Those tokens are black at 0.08
            // alpha, which is a diffuse haze — on the themed chrome behind this
            // composer it reads as a smudge rather than a cast shadow.
            //
            // Both layers are pushed DOWN rather than spread evenly, because an
            // even shadow reads as a glow: it implies light from everywhere,
            // which is no light at all, and the composer ends up looking fuzzy
            // instead of raised. The offsets (6px, 22px) exceed each layer's
            // negative spread (-4px, -16px), so the cast clears the box on the
            // bottom edge and is pulled in at the top — the asymmetry is what
            // says "lit from above".
            //
            //   0 8px  12px -4px  / 0.34  — contact: tight, near the edge
            //   0 30px 44px -16px / 0.48  — cast: far, wide, and the stronger
            //
            // The far layer carrying more alpha than the near one is
            // deliberate and is what gives depth; the usual instinct is the
            // reverse, which flattens it back out.
            //
            // `animate-composer-shadow` then orbits those offsets clockwise on
            // a slow loop (`composerShadowOrbit`, `index.css`), as though the
            // light above the composer circles the room. The static values here
            // are the orbit's 25% stop, so the animation starts from roughly
            // where the unanimated composer sits rather than jumping on load. The static `shadow-[…]` above is
            // not redundant: it is what `motion-reduce:animate-none` falls back
            // to, so the composer keeps its elevation when the OS asks for less
            // motion and merely stops moving. Keyframes override the utility
            // while the animation runs, which is why the two can coexist.
            //
            // Focus is now carried entirely by the border — 0.35 → 0.90 on the
            // same token, so the edge sharpens rather than changing colour —
            // and `transition` names border-color alone. Animating the shadow
            // as well meant two things moving at once for a single event; with
            // the elevation fixed, the composer stays put and only its outline
            // responds. `duration-200 ease-out` is the settle, and
            // `motion-reduce` drops it for anyone who asked the OS for less
            // motion — the cue still lands, just instantly.
            //
            // `border-ring` on drag is untouched — that state is meant to break
            // the pattern.
            className="border-content-faint/35 focus-within:border-content-faint/90 data-[dragging=true]:border-ring shadow-[0_8px_12px_-4px_rgb(0_0_0/0.34),0_30px_44px_-16px_rgb(0_0_0/0.48)] animate-composer-shadow motion-reduce:animate-none flex w-full cursor-text flex-col gap-2 rounded-(--composer-radius) border bg-(--composer-bg) p-(--composer-padding) transition-[border-color] duration-200 ease-out motion-reduce:transition-none data-[dragging=true]:border-dashed data-[dragging=true]:bg-[color-mix(in_oklab,var(--color-accent)_50%,var(--color-background))]">
            {HostComposerAttachments ? <HostComposerAttachments /> : <ComposerAttachments />}
            {/*
             * Lexical rather than the plain `ComposerPrimitive.Input` textarea,
             * because `/` commands need a rich input: the trigger popover has to
             * anchor to the caret and the accepted command has to become a chip
             * rather than literal text the model would read. `commands` is empty
             * unless the host supplies some, and with none the popover never
             * opens, so a host that wants a plain box still gets one.
             */}
            <LexicalComposerInput
              ref={inputWrapperRef}
              placeholder="Send a message..."
              onInputCapture={event => {
                const target = event.target;
                if (target instanceof HTMLElement) {
                  const text = target.textContent ?? '';
                  globalThis.queueMicrotask(() => aui.composer.setText(text));
                }
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
            // Pinned to `primary-500` rather than left on `variant="default"`.
            // That variant paints `bg-primary`, which `styles/shadcn-tokens.css`
            // aliases to `primary-500` in light but `primary-400` in DARK — a
            // pale sky blue. Its label is `--content-inverted`, which is white
            // in both themes (not actually inverted per theme), so in dark the
            // send button was white-on-pale-blue: washed out, and about 2.4:1,
            // which is below AA for a control. `primary-500` under white is
            // ~4.6:1 and reads as the accent in both themes.
            // Overriding here rather than repointing the dark `--primary`
            // alias: that token backs every `variant="default"` button in the
            // app, and dark-mode-lightens-the-accent is a defensible palette
            // choice to make deliberately, not as a side effect of fixing one
            // button. `cn` is tailwind-merge, so the later `bg-primary-500`
            // replaces the variant's `bg-primary` cleanly.
            <TooltipIconButton
              tooltip="Send message"
              side="bottom"
              type="button"
              variant="default"
              size="icon"
              className="aui-composer-send size-7 rounded-full bg-primary-500 text-content-inverted hover:bg-primary-600"
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
                className="aui-composer-send size-7 rounded-full bg-primary-500 text-content-inverted hover:bg-primary-600"
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
  // `min-h` reserves the bar's height (`pt-1.5` + a `size-6` button = 7.5) so a
  // bar revealed on hover does not shift the transcript, and `-mb` gives that
  // reservation back to the flow so it does not stack on top of the spacing the
  // message group already provides. Both MUST sit on this one element: the `-mb`
  // had drifted onto the root, where it only cancelled that element's own `pb`,
  // leaving the reservation uncompensated — a dead 30px band under every turn.
  //
  // The `-mb` step is `gap-y-6` from the message group, NOT the full `min-h`.
  // The bar is pulled into the inter-message gap and must stay inside it: give
  // back more than the gap and the bar's tail paints over the next message's
  // first line, which sits at the same left inset (`ms-2` here, `px-2` there).
  // So the bar occupies the gap exactly and the turns end up 7.5 apart.
  // Keep this in step with `aui_message-group`'s `gap-y-*`; the pairing is
  // asserted in `thread.actionBarSpacing.test.tsx`.
  const ACTION_BAR_HEIGHT = `-mb-6 min-h-7.5 ${ACTION_BAR_PT}`;
  // The root's own `-mb-7.5 pb-7.5` pair below is PAINT-ONLY and unrelated to
  // the above: `content-visibility:auto` implies `contain: paint`, so `pb`
  // widens the paint box to cover the bar that `-mb` pulls past the content
  // box, and the root's `-mb` cancels that padding again in flow.

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
  // Edit is offered only when the bound runtime can honour it. The
  // external-store adapter supplies `onNew` / `onCancel` and neither `onEdit`
  // nor `setMessages`, so assistant-ui reports `edit: false` and
  // `EditComposer` below never renders — the button was clickable and did
  // nothing (#5897).
  //
  // Gated on the capability rather than hard-coded off, so the affordance
  // appears by itself the day the adapter grows `onEdit`.
  const { canEdit } = useAuiEditCapabilities();

  // Hoisted out of the JSX rather than written as `{canEdit && (…)}` inline: a
  // bare JSX logical expression emits no coverage record on its own line, so
  // `diff-cover` reported the gate as an uncovered changed line even while the
  // v8 report showed the surrounding function fully exercised. As a `const` it
  // is an ordinary statement, instrumented like any other.
  const editAction = canEdit ? (
    <ActionBarPrimitive.Edit asChild>
      <TooltipIconButton tooltip="Edit" className="aui-user-action-edit">
        <PencilIcon />
      </TooltipIconButton>
    </ActionBarPrimitive.Edit>
  ) : null;

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
      {editAction}
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
  // The same defect class as the Edit button above, one step from biting: this
  // is rendered unconditionally at both call sites and is invisible today only
  // because `hideWhenSingleBranch` happens to hold — the adapter implements no
  // `setMessages`, so there is never more than one branch. That is
  // assistant-ui's guard doing the work this app intended to do itself, and it
  // would become a second dead control if the prop ever went away.
  const { canSwitchToBranch } = useAuiEditCapabilities();
  if (!canSwitchToBranch) return null;

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
