'use client';

import { ActivityGroup as DefaultActivityGroup } from '@/components/assistant-ui/activity-group';
import {
  ComposerAddAttachment,
  ComposerAttachments,
  UserMessageAttachments,
} from '@/components/assistant-ui/attachment';
import { ComposerTriggerPopover } from '@/components/assistant-ui/composer-trigger-popover';
import { DirectiveText } from '@/components/assistant-ui/directive-text';
import { ErrorState } from '@/components/assistant-ui/elements/error-state';
import { Image } from '@/components/assistant-ui/elements/image';
import { MessageTiming } from '@/components/assistant-ui/elements/message-timing.aui';
import { ScrollAnchor } from '@/components/assistant-ui/elements/scroll-anchor.aui';
import { StoppedRun } from '@/components/assistant-ui/elements/stopped-run';
import { ToolFallback } from '@/components/assistant-ui/elements/tool-fallback';
import { File } from '@/components/assistant-ui/file';
import { ThreadFollowupSuggestions } from '@/components/assistant-ui/follow-up-suggestions';
import { useComposerFocusOnThreadOpen } from '@/components/assistant-ui/lib/useComposerFocusOnThreadOpen';
import { cn } from '@/components/assistant-ui/lib/utils';
import { MarkdownText } from '@/components/assistant-ui/markdown-text';
import { ComposerQuotePreview, SelectionToolbar } from '@/components/assistant-ui/quote';
import { Reasoning } from '@/components/assistant-ui/reasoning';
import { TooltipIconButton } from '@/components/assistant-ui/tooltip-icon-button';
import { Button } from '@/components/assistant-ui/ui/button';
import { Skeleton } from '@/components/assistant-ui/ui/skeleton';
import { ChatErrorNotice } from '@/features/conversations/aui/ChatErrorNotice';
import { ChatSettingsPanel } from '@/features/conversations/aui/ChatSettingsPanel';
import { ConnectionStateBanner } from '@/features/conversations/aui/ConnectionStateBanner';
import {
  useAuiEditCapabilities,
  useAuiReloadCapability,
} from '@/features/conversations/components/aui/auiThreadState';
import { onComposerPrefill } from '@/features/conversations/tools/mcpUi/composerPrefill';
import { McpWidgetStrip } from '@/features/conversations/tools/mcpUi/McpWidgetStrip';
import { collectMessageWidgets } from '@/features/conversations/tools/mcpUi/messageWidgets';
import { MessageVoiceNote, useMessageVoiceNote } from '@/features/voiceNotes/MessageVoiceNote';
import { useT } from '@/lib/i18n/I18nContext';
import { CHAT_ERROR_METADATA_KEY } from '@/store/threadSlice';
import { fullTimestamp, relativeTime } from '@/utils/relativeTime';
import { useActionBarReload, useMessageError } from '@assistant-ui/core/react';
import {
  ActionBarMorePrimitive,
  ActionBarPrimitive,
  type AssistantState,
  AuiIf,
  BranchPickerPrimitive,
  ComposerPrimitive,
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
import debugFactory from 'debug';
import {
  AlertTriangleIcon,
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
  ThumbsDownIcon,
  ThumbsUpIcon,
  Volume2Icon,
  VolumeXIcon,
} from 'lucide-react';
import {
  type ComponentType,
  createContext,
  type FC,
  type PropsWithChildren,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
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
  MessageTasks?: ComponentType | undefined;
  ActiveTasks?: ComponentType | undefined;
  AssistantMessage?: ComponentType | undefined;
  Welcome?: ComponentType | undefined;
  ToolFallback?: ToolCallMessagePartComponent | undefined;
  /**
   * Wraps one run of reasoning and tool calls — everything between the input
   * and the answer — as a single group. Defaults to `ActivityGroup`.
   */
  ActivityGroup?: ComponentType<PropsWithChildren<{ group: ThreadGroupPart }>> | undefined;
  /**
   * Host-owned disclosure for the source parts emitted after an answer:
   * `url` sources (web fetch/search) and `document` sources (memory
   * citations, `sourceType: 'document'`).
   */
  SourceGroup?: ComponentType<{ sources: readonly SourceItemPart[] }> | undefined;
  /**
   * Extra controls in the composer's action row, to the right of the model
   * selector. A seam rather than a fixed set because what belongs there is
   * host-specific — OpenHuman puts the context-window meter and the thread
   * goal here — and hard-coding either would make this component unusable by
   * anything else.
   */
  ComposerExtras?: ComponentType | undefined;
  /** Host-owned controls rendered in the right action cluster before voice. */
  ComposerRightExtras?: ComponentType | undefined;
  /** Host-owned navigation rail mounted inside the scrolling viewport. */
  ConversationMap?: ComponentType | undefined;
  /** Full-width host content immediately above the composer shell. */
  ComposerHeader?: ComponentType | undefined;
  /**
   * Host-owned progress line for the turn in flight, rendered under the last
   * message while `thread.isRunning`.
   *
   * A seam rather than a fixed widget because `isRunning` is all this file
   * knows: what the model is actually doing right now — reasoning round, active
   * tool, delegated sub-agent — lives in the host's own transport state, and
   * without somewhere to put it a long turn is an unlabelled spinner. The host
   * component returns `null` when it has nothing to say.
   */
  RunningStatus?: ComponentType | undefined;
  /**
   * Host-owned content under the last message while no turn is running — e.g.
   * an "Interrupted" marker for a turn the host knows was cut off. Returns
   * `null` when there is nothing to show.
   */
  TranscriptFooter?: ComponentType | undefined;
  /**
   * Host-owned content rendered after the last message, inside the
   * scrollable transcript — artifact cards from versioned tools that
   * carry no tool-call id and would otherwise be invisible to the
   * inline tool-call renderer. Always mounted (not gated on isRunning).
   */
  AfterMessages?: ComponentType | undefined;
  /**
   * Host-owned artifact cards rendered inside each assistant message's
   * content area (matched by requestId). Unlike AfterMessages which
   * renders once at the end of the transcript, MessageArtifacts renders
   * per-message so cards appear attached to the message that created them.
   */
  MessageArtifacts?: ComponentType | undefined;
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
  /**
   * Host sink for files dropped on the composer or pasted into it.
   *
   * Supplying it also replaces `ComposerPrimitive.AttachmentDropzone` with the
   * equivalent host-driven handlers, because that primitive routes files to the
   * runtime's attachment adapter and refuses the drag outright
   * (`dataTransfer.dropEffect = 'none'`) when the runtime declares no
   * attachment capability — which is every runtime that keeps attachments on
   * the host side, as this app does.
   */
  onComposerFiles?: ((files: FileList | File[] | null) => void | Promise<void>) | undefined;
  /**
   * Whether the host can take files right now (feature enabled, composer
   * unlocked, budget left). Drives the drag affordance only; the host still
   * validates whatever arrives.
   */
  canAcceptComposerFiles?: boolean | undefined;
  /**
   * Host composer that REPLACES the built-in one (and the welcome suggestions
   * that belong to it) in the viewport footer, while the transcript above it
   * stays assistant-ui: the mic-first voice composer, whose input is a
   * push-to-talk button, and the workflow copilot, whose sends are structured
   * builder turns rather than chat turns.
   */
  Composer?: ComponentType | undefined;
  /**
   * Host-owned trigger pickers (`/` commands, `@` mentions), mounted inside the
   * composer's `Unstable_TriggerPopoverRoot` in place of the built-in `/`
   * popover fed by `slashCommands`. A component for the same reason the other
   * slots are: its sources are host behaviour this file should not learn.
   */
  ComposerTriggers?: ComponentType | undefined;
};

export type ThreadProps = {
  components?: ThreadComponents | undefined;
  /** Host-owned model route used for real sends. */
  model?: string | null | undefined;
  /** Updates the host's composer route and selected model metadata. */
  onModelChange?: ((value: string | null, contextWindow?: number | null) => void) | undefined;
  /** Host transport error shown in place of an empty welcome state. */
  loadError?: string | null | undefined;
  /**
   * Host-specific Escape behavior (for example cancel + restore prompt).
   * Returning `false` means it did nothing, and the key is left to anything
   * else listening (an open popover); any other return swallows it.
   */
  onEscape?: (() => boolean | void) | undefined;
  /**
   * ArrowUp in an EMPTY composer (no modifiers, no IME): recall the last
   * prompt. Returns whether it did; only then is the key consumed.
   */
  onRecallLastPrompt?: (() => boolean) | undefined;
  /** Placeholder override for the composer input (run / waiting states). */
  composerPlaceholder?: string | undefined;
  /**
   * Commands offered when the composer input starts with `/`. Supplied by the
   * host because a command's `execute` is host behaviour (`/clear` has to
   * reach a runtime this component does not own).
   */
  slashCommands?: readonly Unstable_SlashCommand[] | undefined;
};

/**
 * Whether Lexical's own `SyncPlugin` is driving the composer store, making the
 * host's DOM→store bridge below not merely redundant but harmful.
 *
 * Lexical reconciles from `beforeinput` and needs `getTargetRanges()` to know
 * what the event will change. jsdom implements neither, so there the plugin
 * never commits editor state and the bridge is the ONLY path from a synthetic
 * `input` to the store — which is exactly why #5763 gated that bridge rather
 * than deleting it, and why 54 composer tests depend on it.
 *
 * In a real browser the plugin does commit, and then the bridge's write moves
 * the store through the *external* path. `SyncPlugin`'s runtime subscription
 * reads that as a foreign edit, calls `root.clear()` and rebuilds the editor —
 * and the rebuild restores the caret to the offset captured from the editor
 * state, which still lags the DOM by one keystroke. So the caret never
 * advances: typing `hello` one key at a time produced `holle` with the caret
 * stuck at 1 (#6163).
 *
 * Feature-detected rather than `import.meta.env` because the condition is a
 * real capability, not a build mode: any environment that reconciles from
 * `beforeinput` must not be bridged, and any that cannot must be.
 */
const lexicalDrivesTheStore = (): boolean =>
  typeof InputEvent !== 'undefined' && 'getTargetRanges' in InputEvent.prototype;

// Counts only — never a filename, a MIME type or clipboard content.
const debug = debugFactory('openhuman:assistant-composer');

/**
 * The files a drop is actually carrying.
 *
 * `dataTransfer.files` is the obvious source and is empty more often than it
 * looks: several macOS drag sources — the floating screenshot thumbnail among
 * them — hand the webview promise-backed items instead, leaving `files` at
 * length 0 while `items` holds the same content. Reading `files` alone made
 * those drops do nothing at all, with no error, because there was nothing to
 * reject.
 */
function filesFromDrop(dataTransfer: DataTransfer | null): File[] {
  const direct = Array.from(dataTransfer?.files ?? []);
  if (direct.length > 0) return direct;
  return Array.from(dataTransfer?.items ?? [])
    .filter(item => item.kind === 'file')
    .map(item => item.getAsFile())
    .filter((file): file is File => file !== null);
}

/**
 * Host-driven file drop for the whole open thread, not just the composer box:
 * a file dropped anywhere over the transcript lands as a composer attachment.
 * Mirrors the legacy composer's handlers (`ChatComposer.tsx`) and feeds the
 * same host path as the picker and paste, whose validator decides what the
 * active model can take (images only with vision, documents text-extracted).
 *
 * `preventDefault` on a *file* drag happens whether or not ingest is allowed:
 * without it the webview navigates away to the dropped file and the whole chat
 * is gone. Outside a thread, `installFileDropGuard` refuses the drop instead.
 */
function useThreadFileDrop() {
  const { onComposerFiles, canAcceptComposerFiles } = useContext(ThreadComponentsContext);
  const [isDraggingFiles, setIsDraggingFiles] = useState(false);
  // Attachment validation updates host state asynchronously. Keep drops in
  // arrival order so a second batch cannot validate against stale attachments
  // or overwrite the first batch while it is still being processed.
  const ingestQueueRef = useRef<Promise<void>>(Promise.resolve());

  const isFileDrag = (event: React.DragEvent) =>
    Array.from(event.dataTransfer?.types ?? []).includes('Files');
  const onDragOver = (event: React.DragEvent) => {
    if (!isFileDrag(event)) return;
    event.preventDefault();
    if (!onComposerFiles || !canAcceptComposerFiles) {
      event.dataTransfer.dropEffect = 'none';
      return;
    }
    event.dataTransfer.dropEffect = 'copy';
    setIsDraggingFiles(true);
  };
  const onDragLeave = (event: React.DragEvent) => {
    // Ignore leave events that bubble while the cursor is still over a child.
    if (event.currentTarget.contains(event.relatedTarget as Node | null)) return;
    setIsDraggingFiles(false);
  };
  const onDrop = (event: React.DragEvent) => {
    if (!isFileDrag(event)) return;
    event.preventDefault();
    setIsDraggingFiles(false);
    if (!onComposerFiles || !canAcceptComposerFiles) {
      debug('[assistant-composer] drop: refused, ingest not accepting');
      return;
    }
    const files = filesFromDrop(event.dataTransfer);
    if (files.length === 0) {
      debug('[assistant-composer] drop: file drag carried no readable files');
      return;
    }
    debug('[assistant-composer] drop: queueing %d file(s) for ingest', files.length);
    ingestQueueRef.current = ingestQueueRef.current
      .catch(() => undefined)
      .then(() => onComposerFiles(files))
      .catch(error => {
        debug('[assistant-composer] drop: file ingest failed: %o', error);
      });
  };

  return { isDraggingFiles, dropHandlers: { onDragOver, onDragLeave, onDrop } };
}

const EMPTY_COMPONENTS: ThreadComponents = {};

const ThreadComponentsContext = createContext<ThreadComponents>(EMPTY_COMPONENTS);
// Composer callbacks can change on background refreshes. Keep message renderers
// in a separate context so that change does not invalidate the full transcript.
type MessageComponents = Pick<
  ThreadComponents,
  | 'AssistantMessage'
  | 'ToolFallback'
  | 'ActivityGroup'
  | 'SourceGroup'
  | 'MessageTasks'
  | 'MessageArtifacts'
>;
const MessageComponentsContext = createContext<MessageComponents>(EMPTY_COMPONENTS);

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

const ThreadHistorySkeleton: FC = () => {
  const { t } = useT();
  return (
    <div
      data-slot="aui_thread-history-skeleton"
      role="status"
      className="animate-in fade-in fill-mode-both flex flex-col gap-y-6 [animation-delay:150ms] [animation-duration:200ms]">
      <span className="sr-only">
        {t('assistantUi.thread.loadingConversation', 'Loading conversation')}
      </span>
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
};

export const Thread: FC<ThreadProps> = ({
  components = EMPTY_COMPONENTS,
  model = 'hint:chat',
  onModelChange,
  loadError = null,
  onEscape,
  onRecallLastPrompt,
  composerPlaceholder,
  slashCommands = NO_SLASH_COMMANDS,
}) => {
  const isEmpty = useAuiState(isNewChatView);
  const messageComponents = useMemo<MessageComponents>(
    () => ({
      AssistantMessage: components.AssistantMessage,
      ToolFallback: components.ToolFallback,
      ActivityGroup: components.ActivityGroup,
      SourceGroup: components.SourceGroup,
      MessageTasks: components.MessageTasks,
      MessageArtifacts: components.MessageArtifacts,
    }),
    [
      components.AssistantMessage,
      components.ToolFallback,
      components.ActivityGroup,
      components.SourceGroup,
      components.MessageTasks,
      components.MessageArtifacts,
    ]
  );

  return (
    <ThreadComponentsContext.Provider value={components}>
      <MessageComponentsContext.Provider value={messageComponents}>
        <SlashCommandsContext.Provider value={slashCommands}>
          <ThreadRoot
            isEmpty={isEmpty}
            model={model}
            onModelChange={onModelChange}
            loadError={loadError}
            onEscape={onEscape}
            onRecallLastPrompt={onRecallLastPrompt}
            composerPlaceholder={composerPlaceholder}
          />
        </SlashCommandsContext.Provider>
      </MessageComponentsContext.Provider>
    </ThreadComponentsContext.Provider>
  );
};

const ThreadRoot: FC<{
  isEmpty: boolean;
  model: string | null;
  onModelChange?: (value: string | null, contextWindow?: number | null) => void;
  loadError: string | null;
  onEscape?: () => boolean | void;
  onRecallLastPrompt?: () => boolean;
  composerPlaceholder?: string;
}> = ({
  isEmpty,
  model,
  onModelChange,
  loadError,
  onEscape,
  onRecallLastPrompt,
  composerPlaceholder,
}) => {
  const { t } = useT();
  const {
    Welcome = ThreadWelcome,
    Composer: HostComposer,
    ConversationMap,
    ActiveTasks,
  } = useContext(ThreadComponentsContext);
  const { isDraggingFiles, dropHandlers } = useThreadFileDrop();
  return (
    <ThreadPrimitive.Root
      className="aui-root aui-thread-root bg-background @container flex h-full flex-col"
      {...dropHandlers}
      style={{
        ['--thread-max-width' as string]: '44rem',
        ['--composer-bg' as string]: 'color-mix(in oklab, var(--color-muted) 30%, transparent)',
        ['--composer-radius' as string]: '1rem',
        ['--composer-padding' as string]: '8px',
      }}>
      <ThreadPrimitive.Viewport
        autoScroll
        turnAnchor="bottom"
        scrollToBottomOnRunStart={false}
        data-slot="aui_thread-viewport"
        className="relative flex flex-1 flex-col overflow-x-auto overflow-y-scroll">
        {ConversationMap ? <ConversationMap /> : null}
        <div
          className={cn(
            'mx-auto flex w-full max-w-(--thread-max-width) flex-1 flex-col px-4 pt-4',
            isEmpty && 'justify-center'
          )}>
          {loadError ? (
            <div className="flex flex-1 flex-col items-center justify-center gap-2 text-center">
              <p className="text-sm font-medium text-destructive">
                {t('chat.failedToLoadMessages', 'Failed to load messages')}
              </p>
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
            <RunningStatusSlot />
            <TranscriptFooterSlot />
            <AfterMessagesSlot />
            {ActiveTasks && <ActiveTasks />}
          </div>

          <ThreadPrimitive.ViewportFooter
            className={cn(
              'aui-thread-viewport-footer bg-background flex flex-col gap-4 overflow-visible pb-4 md:pb-6',
              !isEmpty && 'sticky bottom-0 mt-auto rounded-t-(--composer-radius)'
            )}>
            <ThreadScrollToBottom />
            <ThreadFollowupSuggestions />
            <ConnectionStateBanner />
            {HostComposer ? (
              <HostComposer />
            ) : (
              <>
                <Composer
                  model={model}
                  onModelChange={onModelChange}
                  onEscape={onEscape}
                  onRecallLastPrompt={onRecallLastPrompt}
                  placeholder={composerPlaceholder}
                  isDraggingFiles={isDraggingFiles}
                />
                <AuiIf condition={s => isNewChatView(s) && s.composer.isEmpty}>
                  <ThreadSuggestions />
                </AuiIf>
              </>
            )}
          </ThreadPrimitive.ViewportFooter>
        </div>
      </ThreadPrimitive.Viewport>

      {/*
       * Select text in any message and a floating "Quote" button appears over
       * the selection; clicking it drops the excerpt into the composer.
       *
       * It lives OUTSIDE the viewport on purpose: it portals itself to the
       * selection's screen position, so nesting it inside the scroller would
       * only give it a clipped, scrolling ancestor for no benefit. It finds the
       * message by the `data-message-id` that `MessagePrimitive.Root` already
       * emits, so neither message component needed changing.
       */}
      {!HostComposer && <SelectionToolbar />}
    </ThreadPrimitive.Root>
  );
};

/**
 * The host's `RunningStatus`, gated on the thread actually running.
 *
 * Kept inside the message group so the line sits under the last message —
 * where the answer is about to appear — rather than pinned to the composer.
 */
const RunningStatusSlot: FC = () => {
  const { RunningStatus } = useContext(ThreadComponentsContext);
  if (!RunningStatus) return null;
  return (
    <AuiIf condition={s => s.thread.isRunning}>
      <RunningStatus />
    </AuiIf>
  );
};

/** Host footer under the last message, shown only while nothing is running. */
const TranscriptFooterSlot: FC = () => {
  const { TranscriptFooter } = useContext(ThreadComponentsContext);
  if (!TranscriptFooter) return null;
  return (
    <AuiIf condition={s => !s.thread.isRunning}>
      <TranscriptFooter />
    </AuiIf>
  );
};

/** Host content after the last message; always mounted (not gated). */
const AfterMessagesSlot: FC = () => {
  const { AfterMessages } = useContext(ThreadComponentsContext);
  if (!AfterMessages) return null;
  return <AfterMessages />;
};

const ThreadMessage: FC = () => {
  const { AssistantMessage: AssistantMessageComponent = AssistantMessage } =
    useContext(MessageComponentsContext);
  const role = useAuiState(s => s.message.role);
  const isEditing = useAuiState(s => s.message.composer.isEditing);

  if (isEditing) return <EditComposer />;
  if (role === 'user') return <UserMessage />;
  return <AssistantMessageComponent />;
};

const ThreadScrollToBottom: FC = () => {
  const { t } = useT();
  return <ScrollAnchor label={t('chat.message.scrollToBottom')} />;
};

const ChannelMessageActivity: FC = () => {
  const metadata = useAuiState(state => state.message.metadata?.custom) as
    | { extraMetadata?: Record<string, unknown> }
    | undefined;
  const activity = metadata?.extraMetadata?.channelActivity;
  if (!activity || typeof activity !== 'object') return null;
  const value = activity as {
    reactions?: Array<{ value?: unknown; customEmojiId?: unknown }> | Record<string, unknown>;
    readReceipts?: unknown[] | Record<string, unknown>;
  };
  const storedItems = (items: unknown): unknown[] => {
    if (Array.isArray(items)) return items;
    if (items && typeof items === 'object') return Object.values(items);
    return [];
  };
  const reactionCounts = new Map<string, number>();
  for (const reaction of storedItems(value.reactions)) {
    if (!reaction || typeof reaction !== 'object') continue;
    const item = reaction as { value?: unknown };
    if (typeof item.value !== 'string' || item.value.length === 0) continue;
    reactionCounts.set(item.value, (reactionCounts.get(item.value) ?? 0) + 1);
  }
  const readCount = storedItems(value.readReceipts).length;
  if (reactionCounts.size === 0 && readCount === 0) return null;
  return (
    <div data-slot="aui_channel-message-activity" className="mt-1 flex flex-wrap gap-1">
      {[...reactionCounts].map(([reaction, count]) => (
        <span
          key={reaction}
          data-testid="channel-message-reaction"
          className="bg-muted text-muted-foreground rounded-full px-2 py-0.5 text-xs">
          {reaction} {count}
        </span>
      ))}
      {readCount > 0 && (
        <span
          data-testid="channel-message-read-receipts"
          aria-hidden="true"
          className="text-muted-foreground px-1 py-0.5 text-xs">
          ✓✓ {readCount}
        </span>
      )}
    </div>
  );
};

const ThreadWelcome: FC = () => {
  const { t } = useT();
  return (
    <div className="aui-thread-welcome-root mb-6 flex flex-col px-2">
      <h1 className="aui-thread-welcome-message-inner fade-in slide-in-from-bottom-1 animate-in fill-mode-both text-2xl font-medium tracking-tight duration-200">
        {t('chat.newWindowPrompt', 'How can I help you today?')}
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
          <SuggestionPrimitive.Description className="aui-thread-welcome-suggestion-text-2 text-muted-foreground empty:hidden" />
        </Button>
      </SuggestionPrimitive.Trigger>
    </div>
  );
};

export function extractComposerPasteFiles(
  clipboardData: DataTransfer | null | undefined
): globalThis.File[] {
  const itemFiles = Array.from(clipboardData?.items ?? [])
    .filter(item => item.kind === 'file')
    .map(item => item.getAsFile())
    .filter((file): file is globalThis.File => file !== null);
  return itemFiles.length > 0 ? itemFiles : Array.from(clipboardData?.files ?? []);
}

const Composer: FC<{
  model: string | null;
  onModelChange?: (value: string | null, contextWindow?: number | null) => void;
  onEscape?: () => boolean | void;
  onRecallLastPrompt?: () => boolean;
  placeholder?: string;
  /** A file drag is over the thread and will land here; see `useThreadFileDrop`. */
  isDraggingFiles: boolean;
}> = ({ model, onModelChange, onEscape, onRecallLastPrompt, placeholder, isDraggingFiles }) => {
  const { t } = useT();
  const messageInputLabel = t('assistantUi.thread.messageInputLabel', 'Message input');
  const aui = useAui();
  const commands = useContext(SlashCommandsContext);
  const slash = unstable_useSlashCommandAdapter({ commands, fallbackIcon: SlashIcon });
  const inputWrapperRef = useRef<HTMLDivElement>(null);
  const {
    ComposerHeader,
    ComposerAttachments: HostComposerAttachments,
    ComposerTriggers: HostComposerTriggers,
    onComposerFiles,
    canAcceptComposerFiles,
  } = useContext(ThreadComponentsContext);
  useEffect(() => {
    const textbox = inputWrapperRef.current?.querySelector<HTMLElement>('[contenteditable="true"]');
    textbox?.setAttribute('aria-label', messageInputLabel);
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
  }, [messageInputLabel]);
  useComposerFocusOnThreadOpen(inputWrapperRef);

  // Set for as long as an IME composition is open. The gate is a ref rather
  // than state because it is read from a microtask, not from a render.
  //
  // Adopted from #5764 (@ligjn), which identified the hazard this closes: the
  // store write below is deferred, and a fast CJK typist can open the next
  // composition before it runs. That stale write would rebuild the editor
  // mid-composition and cancel it -- #5763 again, one composition later.
  const isComposingTextRef = useRef(false);

  useEffect(() => onComposerPrefill(text => aui.composer.setText(text)), [aui]);
  // ArrowUp recall only fires on an empty composer, so a caret move inside a
  // multi-line draft is never hijacked.
  const composerIsEmpty = useAuiState(state => state.composer.text.length === 0);

  // DOM text -> composer store. The text is read at event time; only the write
  // is deferred by a microtask, so the editor has finished applying the event
  // before the store changes under it.
  //
  // The gate is re-checked INSIDE the microtask, not just at event time: a
  // composition that started in between makes this write stale, and dropping it
  // loses nothing, because the DOM is the source of truth and that
  // composition's own commit reads the whole of it.
  // Capture phase, so the media is pulled out and the default cancelled before
  // Lexical's own paste handling turns it into editor content.
  const handlePasteCapture = (event: React.ClipboardEvent) => {
    if (!onComposerFiles) return;
    if (!canAcceptComposerFiles) {
      debug('[assistant-composer] paste: refused, ingest not accepting');
      return;
    }
    const files = extractComposerPasteFiles(event.clipboardData);
    if (files.length === 0) {
      // The overwhelmingly common case: an ordinary text paste. Left for Lexical.
      return;
    }
    event.preventDefault();
    debug('[assistant-composer] paste: ingesting %d media file(s)', files.length);
    onComposerFiles(files);
  };

  const syncComposerFromDom = (target: EventTarget | null) => {
    if (!(target instanceof HTMLElement)) return;
    const text = target.textContent ?? '';
    globalThis.queueMicrotask(() => {
      if (isComposingTextRef.current) return;
      aui.composer.setText(text);
    });
  };

  return (
    <ComposerPrimitive.Unstable_TriggerPopoverRoot>
      <ComposerPrimitive.Root
        className="aui-composer-root relative flex w-full flex-col"
        data-walkthrough="chat-agent-panel">
        {ComposerHeader ? <ComposerHeader /> : null}
        {/*
         * Neutered whenever the host owns file ingest: every handler in the
         * primitive short-circuits on `disabled`, so the thread-wide handlers in
         * `useThreadFileDrop` are the only ones left and the `data-dragging`
         * styling runs off their state. Left enabled otherwise, so a host that does
         * use a runtime attachment adapter keeps the primitive's behaviour.
         */}
        <ComposerPrimitive.AttachmentDropzone asChild disabled={!!onComposerFiles}>
          <div
            data-slot="aui_composer-shell"
            data-dragging={onComposerFiles && isDraggingFiles ? 'true' : undefined}
            onPasteCapture={handlePasteCapture}
            className="border-foreground/10 focus-within:border-foreground/25 data-[dragging=true]:border-ring flex w-full cursor-text flex-col gap-2 rounded-(--composer-radius) border bg-(--composer-bg) p-(--composer-padding) transition-[border-color] data-[dragging=true]:border-dashed data-[dragging=true]:bg-[color-mix(in_oklab,var(--color-accent)_50%,var(--color-background))]">
            {/* Renders only while a quote is set; dismissing it clears the quote. */}
            <ComposerQuotePreview />
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
              placeholder={placeholder ?? t('chat.typeMessage', 'Send a message...')}
              onCompositionStartCapture={() => {
                isComposingTextRef.current = true;
              }}
              onInputCapture={event => {
                // An IME fires `input` per keystroke while the candidate window is
                // still open, and the text on the DOM then is the pre-edit, not the
                // user's input. Writing it into the store re-renders the editor and
                // cancels the composition, so `nihao` + Enter committed as
                // `n ni nihao 你好` (#5763). The keydown guard below already refuses
                // to act mid-composition; this bridge was the one that did not.
                //
                // Two checks, because they catch different things: the ref covers
                // the whole composition from `compositionstart`, and the native flag
                // covers an `input` that arrives without one.
                if (isComposingTextRef.current) return;
                if ('isComposing' in event.nativeEvent && event.nativeEvent.isComposing) {
                  return;
                }
                if (lexicalDrivesTheStore()) return;
                syncComposerFromDom(event.target);
              }}
              onCompositionEndCapture={event => {
                // Re-open the gate before syncing: what the DOM holds now is what the
                // user committed, and it is the store's turn to catch up.
                //
                // Chromium emits a trailing `input` with `isComposing === false` that
                // the handler above picks up; WebKit does not, so on Safari the
                // committed text exists only here. Running in both is harmless -- the
                // second write carries the same string.
                isComposingTextRef.current = false;
                syncComposerFromDom(event.target);
              }}
              onKeyDownCapture={event => {
                const native = event.nativeEvent;
                if (event.key === 'Escape' && onEscape) {
                  // Only swallow the key when the host acted; otherwise an open
                  // `/` or `@` popover still gets to close on it.
                  if (onEscape() !== false) {
                    event.preventDefault();
                    event.stopPropagation();
                  }
                  return;
                }
                if (
                  event.key === 'ArrowUp' &&
                  onRecallLastPrompt &&
                  !event.shiftKey &&
                  !event.altKey &&
                  !event.metaKey &&
                  !event.ctrlKey &&
                  !isComposingTextRef.current &&
                  !native.isComposing &&
                  composerIsEmpty
                ) {
                  if (onRecallLastPrompt()) {
                    event.preventDefault();
                    event.stopPropagation();
                  }
                  return;
                }
                if (
                  isComposingTextRef.current ||
                  native.isComposing ||
                  native.keyCode === 229 ||
                  ('which' in native && native.which === 229)
                ) {
                  event.preventDefault();
                  event.stopPropagation();
                }
              }}
              className="aui-composer-input caret-primary [&_.aui-lexical-placeholder]:text-muted-foreground/60 relative max-h-48 min-h-10 w-full resize-none bg-transparent px-2.5 py-1 text-base leading-6 outline-none [&_.aui-lexical-input]:min-h-lh [&_.aui-lexical-input]:outline-none [&_.aui-lexical-placeholder]:pointer-events-none [&_.aui-lexical-placeholder]:absolute [&_.aui-lexical-placeholder]:top-0 [&_.aui-lexical-placeholder]:right-0 [&_.aui-lexical-placeholder]:left-0 [&_.aui-lexical-placeholder]:truncate [&_.aui-lexical-placeholder]:px-2.5 [&_.aui-lexical-placeholder]:py-1"
              aria-label={messageInputLabel}
            />
            <ComposerAction model={model} onModelChange={onModelChange} />
          </div>
        </ComposerPrimitive.AttachmentDropzone>

        {HostComposerTriggers ? (
          <HostComposerTriggers />
        ) : (
          commands.length > 0 && (
            <ComposerTriggerPopover char="/" {...slash} emptyItemsLabel="No matching commands" />
          )
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
  const { t } = useT();
  const aui = useAui();
  const composerText = useAuiState(state => state.composer.text);
  const {
    ComposerAddAttachment: HostComposerAddAttachment,
    hasComposerAttachments,
    onComposerAttachmentSend,
    ComposerIdleAction,
    ComposerRightExtras,
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
      <div className="flex items-center gap-1.5">
        {HostComposerAddAttachment ? <HostComposerAddAttachment /> : <ComposerAddAttachment />}
        <ChatSettingsPanel model={model} onModelChange={onModelChange} />
        <ComposerExtrasSlot />
      </div>
      <div className="flex items-center gap-1.5">
        {ComposerRightExtras ? <ComposerRightExtras /> : null}
        {onSwitchToMicCloud && (
          <TooltipIconButton
            tooltip={t('composer.voiceMode', 'Voice mode')}
            side="bottom"
            type="button"
            variant="ghost"
            size="icon"
            className="aui-composer-voice-mode text-muted-foreground hover:text-foreground size-7 rounded-full"
            aria-label={t('composer.voiceMode', 'Voice mode')}
            disabled={isRunning}
            onClick={onSwitchToMicCloud}>
            <MicIcon className="aui-composer-dictate-icon size-4" />
          </TooltipIconButton>
        )}
        {/*
          Permanently false, deliberately: `useOpenHumanExternalStore` supplies
          no `adapters.dictation`, and the reasoning for keeping it that way
          lives there. Short version — Web Speech's constructor exists in our
          WKWebView but `start()` never succeeds, and with the speech usage
          strings present it hangs silently rather than erroring, which would
          strand the composer in `dictation != null`. Working dictation already
          ships as the `mic-cloud` composer, whose "Voice mode" button is the
          one directly above this block.
        */}
        <AuiIf condition={s => s.thread.capabilities.dictation}>
          <AuiIf condition={s => s.composer.dictation == null}>
            <ComposerPrimitive.Dictate asChild>
              <TooltipIconButton
                tooltip={t('assistantUi.thread.voiceInput', 'Voice input')}
                side="bottom"
                type="button"
                variant="ghost"
                size="icon"
                className="aui-composer-dictate size-7 rounded-full"
                aria-label={t('assistantUi.thread.startVoiceInput', 'Start voice input')}>
                <MicIcon className="aui-composer-dictate-icon size-4" />
              </TooltipIconButton>
            </ComposerPrimitive.Dictate>
          </AuiIf>
          <AuiIf condition={s => s.composer.dictation != null}>
            <ComposerPrimitive.StopDictation asChild>
              <TooltipIconButton
                tooltip={t('assistantUi.thread.stopDictation', 'Stop dictation')}
                side="bottom"
                type="button"
                variant="ghost"
                size="icon"
                className="aui-composer-stop-dictation text-destructive size-7 rounded-full"
                aria-label={t('assistantUi.thread.stopVoiceInput', 'Stop voice input')}>
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
              tooltip={t('chat.send', 'Send message')}
              side="bottom"
              type="button"
              variant="default"
              size="icon"
              className="aui-composer-send size-7 rounded-full bg-primary-500 text-content-inverted hover:bg-primary-600"
              data-testid="send-message-button"
              aria-label={t('chat.send', 'Send message')}
              onClick={() => {
                onComposerAttachmentSend?.();
                aui.composer.setText('');
              }}>
              <ArrowUpIcon className="aui-composer-send-icon size-4" />
            </TooltipIconButton>
          ) : (
            <ComposerPrimitive.Send asChild>
              <TooltipIconButton
                tooltip={t('chat.send', 'Send message')}
                side="bottom"
                type="button"
                variant="default"
                size="icon"
                className="aui-composer-send size-7 rounded-full bg-primary-500 text-content-inverted hover:bg-primary-600"
                data-testid="send-message-button"
                aria-label={t('chat.send', 'Send message')}>
                <ArrowUpIcon className="aui-composer-send-icon size-4" />
              </TooltipIconButton>
            </ComposerPrimitive.Send>
          )}
        </AuiIf>
        {/*
          While a turn runs, typed text is a queued follow-up (the runtime's
          `queue` capability keeps Send enabled), so the primary slot offers to
          queue it; Stop takes the slot only when there is nothing to send —
          OpenClaw's rule, and the one that keeps a half-typed follow-up one
          Enter away instead of behind a Stop button.
        */}
        <AuiIf condition={s => s.thread.isRunning && s.composer.text.trim().length > 0}>
          <ComposerPrimitive.Send asChild>
            <TooltipIconButton
              tooltip={t('composer.queueSend')}
              side="bottom"
              type="button"
              variant="default"
              size="icon"
              className="aui-composer-send size-7 rounded-full bg-primary-500 text-content-inverted hover:bg-primary-600"
              data-testid="queue-message-button"
              data-analytics-id="chat-composer-queue-send"
              aria-label={t('composer.queueSend')}>
              <ArrowUpIcon className="aui-composer-send-icon size-4" />
            </TooltipIconButton>
          </ComposerPrimitive.Send>
        </AuiIf>
        <AuiIf condition={s => s.thread.isRunning && s.composer.text.trim().length === 0}>
          <ComposerPrimitive.Cancel asChild>
            <TooltipIconButton
              tooltip={t('composer.stopEsc')}
              side="bottom"
              type="button"
              variant="default"
              size="icon"
              className="aui-composer-cancel size-7 rounded-full bg-primary-500 text-content-inverted hover:bg-primary-600"
              data-testid="stop-generation-button"
              data-analytics-id="chat-composer-stop"
              aria-label={t('chat.stopGeneration', 'Stop generating')}>
              <SquareIcon className="aui-composer-cancel-icon size-3.5 fill-current" />
            </TooltipIconButton>
          </ComposerPrimitive.Cancel>
        </AuiIf>
      </div>
    </div>
  );
};

/**
 * The runtime's error presentation for a failed message. The external-store
 * projection marks persisted `chat_error` rows as incomplete/error, so they
 * use this same card as errors raised directly by assistant-ui.
 *
 * `useMessageError` gives us the raw error value for the card. Failed turns
 * have no committed assistant reply id for `threads.regenerate`, so this card
 * must not offer Reload even when the runtime supports it for settled replies.
 */
const MessageError: FC = () => {
  const { t } = useT();
  const error = useMessageError();
  if (error === undefined) return null;
  const detail = typeof error === 'string' ? error : JSON.stringify(error);
  return (
    <MessagePrimitive.Error>
      <ErrorState
        className="aui-message-error-root mt-2"
        title={t('misc.somethingWentWrong', 'Something went wrong')}
        detail={detail}
        retrying={false}
      />
    </MessagePrimitive.Error>
  );
};

/** A URL `source` part, e.g. a web fetch/search result. */
export type SourceUrlPart = { id: string; sourceType: 'url'; url: string; title?: string };
/** A document `source` part, e.g. a memory citation. */
export type SourceDocumentPart = { id: string; sourceType: 'document'; title?: string };
/** Either kind of `source` part this app emits. */
export type SourceItemPart = SourceUrlPart | SourceDocumentPart;

const selectMessageParts = (state: AssistantState) => state.message.parts;

/** Gives the host all source parts (`url` and `document`) represented by one grouped source node. */
const SourceGroupSlot: FC<{ Component: ComponentType<{ sources: readonly SourceItemPart[] }> }> = ({
  Component,
}) => {
  const parts = useAuiState(selectMessageParts);
  const sources = parts.flatMap((part): SourceItemPart[] => {
    if (part.type !== 'source') return [];
    if (part.sourceType === 'url') {
      return [
        {
          id: part.id,
          sourceType: 'url',
          url: part.url,
          ...(part.title ? { title: part.title } : {}),
        },
      ];
    }
    if (part.sourceType === 'document') {
      return [
        { id: part.id, sourceType: 'document', ...(part.title ? { title: part.title } : {}) },
      ];
    }
    return [];
  });
  return sources.length > 0 ? <Component sources={sources} /> : null;
};

/** Whether this message is a stopped/cancelled turn's partial reply. */
const isStoppedRun = (s: AssistantState): boolean =>
  s.message.status?.type === 'incomplete' && s.message.status.reason === 'cancelled';

/**
 * `cancel_reason` (wire-contract.md `chat_cancelled`, carried through
 * `metadata.custom.extraMetadata` by `assistantUiMessages.ts`), so the reason
 * chip can tell a user-initiated Stop from a turn the core superseded.
 */
const selectStoppedRunCancelReason = (s: AssistantState): string | undefined => {
  const custom = s.message.metadata?.custom as
    | { extraMetadata?: { cancelReason?: string; supersededBy?: string } }
    | undefined;
  return custom?.extraMetadata?.cancelReason;
};

/**
 * Renders in place of the normal part switch for a stopped/cancelled
 * assistant message (#4862 kept the raw text visible via a plain "Stopped"
 * label; this replaces that with the real vendored element). Continue re-runs
 * the turn through the same Reload capability `AssistantActionBar` uses;
 * Discard drops the partial reply from this client's view via `onDelete`
 * (`useOpenHumanExternalStore.ts` — the core keeps the persisted row, this
 * only stops showing it here).
 */
const StoppedRunSlot: FC = () => {
  const aui = useAui();
  const { t } = useT();
  const cancelReason = useAuiState(selectStoppedRunCancelReason);
  const { disabled: reloadDisabled, reload } = useActionBarReload();
  const reasonLabel =
    cancelReason === 'superseded'
      ? t('conversations.assistantUi.stoppedRun.reasonSuperseded')
      : t('conversations.assistantUi.stoppedRun.reasonUserStop');
  return (
    <StoppedRun
      data-testid="stopped-marker"
      reason={reasonLabel}
      onContinue={() => {
        if (!reloadDisabled) reload();
      }}
      onDiscard={() => aui.message.delete()}
      continueLabel={t('common.continue')}
      discardLabel={t('settings.ai.discard')}
    />
  );
};

const AssistantMessage: FC = () => {
  const voiceNote = useMessageVoiceNote();
  const {
    ToolFallback: ToolFallbackComponent = ToolFallback,
    ActivityGroup = DefaultActivityGroup,
    SourceGroup,
    MessageTasks,
    MessageArtifacts,
  } = useContext(MessageComponentsContext);
  const stopped = useAuiState(isStoppedRun);
  const parts = useAuiState(s => s.message.parts);
  const widgets = useMemo(() => collectMessageWidgets(parts), [parts]);

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
  // the above: the padding reserves paint space for the action bar, and the
  // negative margin compensates for that padding in normal flow.

  return (
    <MessagePrimitive.Root
      data-slot="aui_assistant-message-root"
      data-role="assistant"
      data-testid="agent-message"
      className="fade-in slide-in-from-bottom-1 animate-in relative -mb-7.5 pb-7.5 duration-150">
      {/*
       * One vertical rhythm for the whole message, rather than each part
       * bringing its own margin. Measured before this change the gaps ran
       * 16 / 0 / 0 / 16 / 0 / 0 px — `reasoning-root` carries `mb-4` and
       * nothing else carried anything, so a reasoning block sat apart while a
       * tool group and the prose beneath it touched. `[&>*+*]:mt-3` spaces
       * adjacent blocks evenly and the `mb-0` override neutralises the one
       * component with an opinion.
       *
       * Reasoning and tool calls share ONE group per run, in the order they
       * happened, so a turn reads input → work → answer. Splitting them into
       * reasoning and tool sub-groups turned an interleaved turn (think, call,
       * think, call) into a stack of unrelated collapsibles.
       */}
      <div
        data-slot="aui_assistant-message-content"
        className="text-foreground px-2 leading-relaxed wrap-break-word">
        <MessagePrimitive.GroupedParts
          groupBy={groupPartByType({
            reasoning: ['group-activity'],
            'tool-call': ['group-activity'],
            'standalone-tool-call': [],
            source: ['group-source'],
          })}>
          {({ part, children }) => {
            switch (part.type) {
              case 'group-activity': {
                const groupWidgets = widgets.filter(widget =>
                  part.indices.includes(widget.partIndex)
                );
                return (
                  <>
                    <ActivityGroup group={part}>{children}</ActivityGroup>
                    <McpWidgetStrip widgets={groupWidgets} />
                  </>
                );
              }
              case 'group-source':
                return SourceGroup ? <SourceGroupSlot Component={SourceGroup} /> : null;
              case 'text':
                // A stopped turn's partial is markdown like any reply (#7729);
                // `StoppedRunSlot` below adds only the reason and actions.
                return <MarkdownText />;
              case 'reasoning':
                // A step inside the activity group, not a disclosure of its own.
                return (
                  <div
                    data-slot="aui_activity-reasoning"
                    className="text-muted-foreground border-border border-s-2 ps-3 text-sm leading-relaxed">
                    <Reasoning {...part} />
                  </div>
                );
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
                // The host RunningStatus slot renders the single shared loading
                // state below the message group. Rendering assistant-ui's raw
                // indicator part as well produces a second, disconnected dot.
                return null;
              default:
                return null;
            }
          }}
        </MessagePrimitive.GroupedParts>
        {stopped && <StoppedRunSlot />}
        <MessageError />
        <ChatErrorNotice />
        {MessageTasks && (
          <div>
            <MessageTasks />
          </div>
        )}
        {MessageArtifacts && (
          <div className="mt-2">
            <MessageArtifacts />
          </div>
        )}
        {voiceNote ? (
          <div data-slot="aui_assistant-message-voice-note" className="mt-2">
            <MessageVoiceNote note={voiceNote} withTranscript={false} />
          </div>
        ) : null}
        <ChannelMessageActivity />
      </div>

      <div
        data-slot="aui_assistant-message-footer"
        className={cn('ms-2 flex items-center', ACTION_BAR_HEIGHT)}>
        <BranchPicker />
        <AssistantActionBar />
      </div>
    </MessagePrimitive.Root>
  );
};

const AssistantActionBar: FC = () => {
  const { t } = useT();
  // assistant-ui's own disabled predicate for Reload is
  // `isRunning || isDisabled || role !== 'assistant'` — it never consults
  // `capabilities.reload`, so the button ships enabled on every settled
  // assistant message while the external-store adapter supplies no `onReload`
  // and the runtime throws on click.
  //
  // Hoisted to a `const` rather than written inline for the same coverage
  // reason as `editAction` in `UserActionBar`.
  const canReload = useAuiReloadCapability();
  const isFailedTurn = useAuiState(s => {
    if (s.message.status?.type === 'incomplete' && s.message.status.reason === 'error') return true;
    const custom = s.message.metadata?.custom as
      | { extraMetadata?: Record<string, unknown> }
      | undefined;
    return custom?.extraMetadata?.[CHAT_ERROR_METADATA_KEY] !== undefined;
  });
  const reloadAction =
    canReload && !isFailedTurn ? (
      <ActionBarPrimitive.Reload asChild>
        <TooltipIconButton tooltip={t('chat.message.refresh')}>
          <RefreshCwIcon />
        </TooltipIconButton>
      </ActionBarPrimitive.Reload>
    ) : null;

  return (
    <ActionBarPrimitive.Root
      hideWhenRunning
      autohide="not-last"
      className="aui-assistant-action-bar-root text-muted-foreground animate-in fade-in col-start-3 row-start-2 -ms-1 flex gap-1 duration-200">
      <ActionBarPrimitive.Copy asChild>
        <TooltipIconButton tooltip={t('chat.message.copy')}>
          <AuiIf condition={s => s.message.isCopied}>
            <CheckIcon className="animate-in zoom-in-50 fade-in duration-200 ease-out" />
          </AuiIf>
          <AuiIf condition={s => !s.message.isCopied}>
            <CopyIcon className="animate-in zoom-in-75 fade-in duration-150" />
          </AuiIf>
        </TooltipIconButton>
      </ActionBarPrimitive.Copy>
      {reloadAction}
      {/* Thumbs render only because the external store now supplies
          `adapters.feedback`; the runtime gates them on that key alone. The
          pressed state comes from `message.submittedFeedback`, which our message
          converter re-emits from the persisted rating — see the Defect A note
          there, without which a pressed thumb silently un-presses on the next
          store update.

          Deliberately NOT gated the way `reloadAction` above is. That gate
          exists because assistant-ui's Reload ignores `capabilities.reload` and
          the runtime *throws* on click when the adapter supplies no `onReload`.
          These primitives instead compute `disabled = disabled || !callback`
          from the adapter's own hook, so with no adapter they render disabled
          rather than throwing — and we supply `adapters.feedback`
          unconditionally, so they are always live here. */}
      <ActionBarPrimitive.FeedbackPositive asChild>
        <TooltipIconButton
          tooltip={t('chat.message.goodResponse')}
          data-testid="assistant-feedback-positive"
          className="data-[submitted=true]:text-primary-600 dark:data-[submitted=true]:text-primary-400">
          <ThumbsUpIcon />
        </TooltipIconButton>
      </ActionBarPrimitive.FeedbackPositive>
      <ActionBarPrimitive.FeedbackNegative asChild>
        <TooltipIconButton
          tooltip={t('chat.message.badResponse')}
          data-testid="assistant-feedback-negative"
          className="data-[submitted=true]:text-coral-600 dark:data-[submitted=true]:text-coral-400">
          <ThumbsDownIcon />
        </TooltipIconButton>
      </ActionBarPrimitive.FeedbackNegative>
      {/*
       * Read aloud, through the same TTS the chat mascot uses. Gated on the
       * capability rather than rendered unconditionally: `actionBarSpeakDisabled`
       * checks only the message's role and running status, NOT
       * `capabilities.speech`, so an ungated Speak button on a runtime with no
       * `adapters.speech` is enabled, clickable, and throws. The gate makes the
       * control appear exactly when it can work — the same rule `UserActionBar`
       * applies to Edit (#5897).
       */}
      <AuiIf condition={s => s.thread.capabilities.speech}>
        <AuiIf condition={s => s.message.speech == null}>
          <ActionBarPrimitive.Speak asChild>
            <TooltipIconButton tooltip={t('chat.message.readAloud')}>
              <Volume2Icon />
            </TooltipIconButton>
          </ActionBarPrimitive.Speak>
        </AuiIf>
        <AuiIf condition={s => s.message.speech != null}>
          <ActionBarPrimitive.StopSpeaking asChild>
            <TooltipIconButton tooltip={t('chat.message.stopReading')}>
              <VolumeXIcon className="text-destructive" />
            </TooltipIconButton>
          </ActionBarPrimitive.StopSpeaking>
        </AuiIf>
      </AuiIf>
      <ActionBarMorePrimitive.Root>
        <ActionBarMorePrimitive.Trigger asChild>
          <TooltipIconButton
            tooltip={t('chat.message.more')}
            className="data-[state=open]:bg-accent">
            <MoreHorizontalIcon />
          </TooltipIconButton>
        </ActionBarMorePrimitive.Trigger>
        <ActionBarMorePrimitive.Content
          side="bottom"
          align="start"
          sideOffset={6}
          className="aui-action-bar-more-content bg-popover text-popover-foreground data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95 data-[state=open]:animate-in data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-95 data-[state=closed]:animate-out data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2 z-50 min-w-[8rem] overflow-hidden rounded-xl border p-1.5">
          <ActionBarPrimitive.ExportMarkdown asChild>
            <ActionBarMorePrimitive.Item className="aui-action-bar-more-item hover:bg-accent hover:text-accent-foreground focus:bg-accent focus:text-accent-foreground flex cursor-pointer items-center gap-2 rounded-lg px-2.5 py-1.5 text-sm outline-none select-none">
              <DownloadIcon className="size-4" />
              {t('assistantUi.thread.exportAsMarkdown', 'Export as Markdown')}
            </ActionBarMorePrimitive.Item>
          </ActionBarPrimitive.ExportMarkdown>
        </ActionBarMorePrimitive.Content>
      </ActionBarMorePrimitive.Root>
      {/*
       * Renders nothing until the stream completes and `chat_done.timing`
       * lands on `message.metadata.timing` (`assistantUiMessages.ts`); see
       * that element's own docstring for why it belongs inside this root.
       */}
      <MessageTiming />
      <MessageTimestamp />
    </ActionBarPrimitive.Root>
  );
};

/**
 * Muted relative send time beside the message actions ("5m ago"), with the
 * full local date, time and zone in its tooltip. The action bar mounts on
 * hover (and for the last message), so "now" is taken when it mounts rather
 * than ticking on every message in a long transcript.
 */
const MessageTimestamp: FC = () => {
  const { t, locale } = useT();
  const createdAt = useAuiState(s => s.message.createdAt);
  const [now] = useState(() => new Date());
  if (!(createdAt instanceof Date)) return null;
  const relative = relativeTime(createdAt, now, locale);
  if (!relative) return null;
  const label =
    relative.kind === 'justNow'
      ? t('chat.message.justNow')
      : relative.kind === 'minutes'
        ? t('chat.message.minutesAgo').replace('{count}', String(relative.count))
        : relative.kind === 'hours'
          ? t('chat.message.hoursAgo').replace('{count}', String(relative.count))
          : relative.label;
  return (
    <time
      data-testid="message-timestamp"
      dateTime={createdAt.toISOString()}
      title={fullTimestamp(createdAt, locale)}
      className="text-muted-foreground/80 ms-1 self-center text-xs tabular-nums">
      {label}
    </time>
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
  const voiceNote = useMessageVoiceNote();
  return (
    <MessagePrimitive.Root
      data-slot="aui_user-message-root"
      className="fade-in slide-in-from-bottom-1 animate-in grid auto-rows-auto grid-cols-[minmax(72px,1fr)_auto] content-start gap-y-2 px-2 duration-150 [&:where(>*)]:col-start-2"
      data-role="user">
      <UserMessageAttachments />

      <div className="aui-user-message-content-wrapper relative col-start-2 min-w-0">
        <div className="aui-user-message-content peer bg-muted text-foreground rounded-(--composer-radius) px-4 py-2 wrap-break-word empty:hidden">
          {/* `Text: DirectiveText` because the composer can put directive syntax
              into a user message without anyone opting in. The `/` popover is
              built from `unstable_useSlashCommandAdapter`, which returns an
              `action` behaviour and sets no `removeOnExecute`; the runtime's
              `triggerSelectionResource` then takes `else insertDirective()`,
              replacing the typed `/clear` with `formatter.serialize(item)` —
              `:command[/clear]{name=clear}` — as an audit-trail chip. Without a
              `Text` component here that renders as raw syntax and is sent to the
              model verbatim. Assistant text is unaffected: it renders through
              `MarkdownText` on the part switch below, a different slot. */}
          {voiceNote ? (
            <MessageVoiceNote note={voiceNote} withTranscript />
          ) : (
            <MessagePrimitive.Parts
              components={{ Text: DirectiveText, File: UserFilePart, Image: UserImagePart }}
            />
          )}
        </div>
        <div className="aui-user-action-bar-wrapper absolute start-0 top-1/2 -translate-x-full -translate-y-1/2 pe-2 peer-empty:hidden rtl:translate-x-full">
          <UserActionBar />
        </div>
      </div>

      <BranchPicker
        data-slot="aui_user-branch-picker"
        className="col-span-full col-start-1 -me-1 justify-end"
      />
      <ChannelMessageActivity />
    </MessagePrimitive.Root>
  );
};

const UserActionBar: FC = () => {
  const { t } = useT();
  // The adapter supplies editing; preview runtimes without it hide the action.
  const { canEdit } = useAuiEditCapabilities();

  // Hoisted out of the JSX rather than written as `{canEdit && (…)}` inline: a
  // bare JSX logical expression emits no coverage record on its own line, so
  // `diff-cover` reported the gate as an uncovered changed line even while the
  // v8 report showed the surrounding function fully exercised. As a `const` it
  // is an ordinary statement, instrumented like any other.
  const editAction = canEdit ? (
    <ActionBarPrimitive.Edit asChild>
      <TooltipIconButton tooltip={t('chat.message.edit')} className="aui-user-action-edit">
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
        <TooltipIconButton
          tooltip={t('chat.message.copyMessage')}
          title={t('chat.message.copyMessage')}>
          <CopyIcon />
        </TooltipIconButton>
      </ActionBarPrimitive.Copy>
      {editAction}
    </ActionBarPrimitive.Root>
  );
};

/**
 * How many later turns editing this message would discard — `onEdit`
 * (`useOpenHumanExternalStore.ts`) truncates the thread's single lineage from
 * this message on, exactly like `onReload`, so every message after it (not
 * just its direct reply) is what a Send here throws away. `s.message.index`
 * is the position `MessageState` already tracks;
 * `s.thread.messages.length - 1 - index` is everything after it. Kept as its
 * own primitive-returning selector (a plain number), never combined with
 * `value` below into one object literal — `useAuiState`'s selector is
 * compared by `Object.is`, so an object literal differs from itself on every
 * store tick and free-runs the subscription (the "Maximum update depth
 * exceeded" loop this file hit once already).
 */
const selectDiscardedReplies = (s: AssistantState): number =>
  Math.max(0, s.thread.messages.length - 1 - s.message.index);

const EditComposer: FC = () => {
  const { t } = useT();
  const discardedReplies = useAuiState(selectDiscardedReplies);
  return (
    <MessagePrimitive.Root data-slot="aui_edit-composer-wrapper" className="flex flex-col px-2">
      <ComposerPrimitive.Root
        data-slot="edit-message"
        className="bg-background border-border/60 ms-auto flex w-full flex-col gap-3 rounded-2xl border p-3.5">
        <ComposerPrimitive.Input
          rows={2}
          autoFocus
          aria-label={t('conversations.assistantUi.edit.ariaLabel')}
          className="bg-foreground/[0.04] text-foreground/90 min-h-16 resize-none rounded-xl px-3 py-2.5 text-sm leading-relaxed outline-none focus-visible:ring-1 focus-visible:ring-ring"
        />
        {discardedReplies > 0 && (
          <div className="flex items-center gap-2 text-amber-700 dark:text-amber-400">
            <AlertTriangleIcon aria-hidden className="size-3.5 shrink-0" />
            <span className="font-mono text-[11px] tabular-nums">
              {t(
                discardedReplies === 1
                  ? 'conversations.assistantUi.edit.discardedRepliesOne'
                  : 'conversations.assistantUi.edit.discardedRepliesOther'
              ).replace('{count}', String(discardedReplies))}
            </span>
          </div>
        )}
        <div className="flex items-center justify-end gap-2">
          <ComposerPrimitive.Cancel asChild>
            <Button variant="ghost" size="sm" className="rounded-full">
              {t('common.cancel')}
            </Button>
          </ComposerPrimitive.Cancel>
          <ComposerPrimitive.Send asChild>
            <Button size="sm" className="rounded-full">
              {t('chat.elicitation.send')}
            </Button>
          </ComposerPrimitive.Send>
        </div>
      </ComposerPrimitive.Root>
    </MessagePrimitive.Root>
  );
};

const BranchPicker: FC<BranchPickerPrimitive.Root.Props> = ({ className, ...rest }) => {
  // Show branching only on runtimes whose adapter supports it.
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
