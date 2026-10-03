/**
 * Dev-only gallery of tool-call presentation (`/dev/tools`).
 *
 * Renders the chat's tool-call card and assistant-ui tool timeline with
 * realistic payloads in every state, plus the whole core tool catalog with
 * each tool's icon and both tenses, so a label or icon regression is visible
 * at a glance. Registered only in dev builds (see `AppRoutes.tsx`).
 */
import { BrainIcon, FileIcon, ListChecksIcon, SparklesIcon, WorkflowIcon } from 'lucide-react';
import { useState } from 'react';

import { AgentPlan } from '../../components/assistant-ui/elements/agent-plan';
import { AgentStatus } from '../../components/assistant-ui/elements/agent-status';
import {
  ComposerCommandItem,
  ComposerMenu,
  ComposerMenuItem,
} from '../../components/assistant-ui/elements/composer';
import { ContextBreakdown } from '../../components/assistant-ui/elements/context-breakdown';
import { ContextDisplayRing } from '../../components/assistant-ui/elements/context-display';
import {
  ConversationSearch,
  type SearchHit,
} from '../../components/assistant-ui/elements/conversation-search';
import { CitationMarker } from '../../components/assistant-ui/elements/inline-citation';
import { MemoryChips } from '../../components/assistant-ui/elements/memory-chips';
import { MessageQueue } from '../../components/assistant-ui/elements/message-queue';
import { ScheduleCard } from '../../components/assistant-ui/elements/schedule-card';
import { SettingsPanel } from '../../components/assistant-ui/elements/settings-panel';
import {
  Source,
  SourceIcon,
  SourceTitle,
} from '../../components/assistant-ui/elements/sources.aui';
import { Timeline, type TimelineEvent } from '../../components/assistant-ui/elements/timeline';
import { TodoList } from '../../components/assistant-ui/elements/todo-list';
import { ToolTimeline } from '../../components/assistant-ui/elements/tool-timeline';
import { ApprovalCardAdapter } from '../../features/conversations/aui/ApprovalCardAdapter';
import { ConnectionStateNotice } from '../../features/conversations/aui/ConnectionStateBanner';
import { contextBreakdownSegments } from '../../features/conversations/aui/ContextUsage';
import { ElicitationAdapter } from '../../features/conversations/aui/ElicitationAdapter';
import { PermissionGrantAdapter } from '../../features/conversations/aui/PermissionGrantAdapter';
import { PlanReviewCardCore } from '../../features/conversations/aui/PlanReviewPart';
import { AssistantUiToolCallCard } from '../../features/conversations/components/AssistantUiToolCall';
import coreToolNames from '../../features/conversations/tools/__fixtures__/coreToolNames.json';
import { ToolIcon } from '../../features/conversations/tools/ToolIcon';
import { describeToolCall, toolLabel } from '../../features/conversations/tools/toolPresentation';
import { useT } from '../../lib/i18n/I18nContext';
import type { PendingApproval } from '../../store/chatRuntimeSlice';
import {
  MOCK_COMMANDS_LIST,
  MOCK_CONNECTION_PHASES,
  MOCK_CONTEXT_BREAKDOWN,
  MOCK_CONTEXT_USAGE,
  MOCK_MEMORY_FETCH,
  MOCK_MESSAGE_QUEUE,
  MOCK_THREAD_FILES,
} from './assistant-ui-demo/assistantUiMock/mockScript';
import { FollowupSuggestionsDemo } from './assistant-ui-demo/FollowupSuggestionsDemo';

/** Icon per `commands_list` kind for the composer menu fixture. */
const COMMAND_KIND_ICONS = {
  builtin: ListChecksIcon,
  skill: SparklesIcon,
  workflow: WorkflowIcon,
} as const;

/** Fixtures for every approval-card state (WS-B, assistant-ui-elements plan). */
const APPROVAL_PENDING_APPROVAL: PendingApproval = {
  requestId: 'dev-approval-pending',
  toolName: 'shell',
  message: 'Run `shell` — list the repository root',
  command: 'ls -la /Users/dev/project',
};

const APPROVAL_EXPIRING: PendingApproval = {
  ...APPROVAL_PENDING_APPROVAL,
  requestId: 'dev-approval-expiring',
  expiresAt: new Date(Date.now() + 65_000).toISOString(),
};

const COMPOSIO_CONNECT_APPROVAL: PendingApproval = {
  requestId: 'dev-composio-connect',
  toolName: 'composio_connect',
  message: 'Connect Google Drive?',
  toolkit: 'googledrive',
};

const SEARCH_RESULT = [
  'Search results for: rust async traits (via Exa)',
  '1. Announcing async fn and return-position impl Trait in traits',
  '   https://blog.rust-lang.org/2023/12/21/async-fn-rpit-in-traits.html',
  '   Published: 2023-12-21',
  '   The Rust Async Working Group is excited to announce major progress.',
  '2. async-trait crate',
  '   https://docs.rs/async-trait/latest/async_trait/',
  '   Type erasure for async trait methods.',
  '3. Async in traits: the design',
  '   https://smallcultfollowing.com/babysteps/blog/2019/10/26/async-fn-in-traits-are-hard/',
  '4. Tokio tutorial',
  '   https://tokio.rs/tokio/tutorial',
].join('\n');

/** Fixtures for WS-G's rich-content elements (sources/citations, memory, schedule, conversation map). */
const MEMORY_SEARCH_HITS: SearchHit[] = [
  {
    id: 'hit-1',
    before: 'The deploy runs ',
    match: 'nightly',
    after: ' at 2am UTC.',
    position: 12,
  },
  {
    id: 'hit-2',
    before: 'Config lives in ',
    match: 'deploy/',
    after: 'config.yaml.',
    position: 68,
  },
];

const MEMORY_TIMELINE_EVENTS: TimelineEvent[] = [
  { id: 'evt-1', when: 'past', time: '09:02', title: 'What is the deploy schedule?' },
  { id: 'evt-2', when: 'past', time: '09:05', title: 'Where does the config live?' },
  { id: 'evt-3', when: 'now', time: '09:11', title: 'Can you add a Friday run?' },
];

const SAMPLES = [
  {
    toolName: 'web_search_tool',
    args: { query: 'rust async traits' },
    result: SEARCH_RESULT,
    status: 'success' as const,
    elapsedMs: 1840,
  },
  {
    toolName: 'web_search_tool',
    args: { query: 'tauri v2 deep links' },
    status: 'running' as const,
  },
  {
    toolName: 'file_read',
    args: { path: 'crates/openhuman-core/src/agent/progress.rs' },
    result: 'pub enum AgentProgress {\n    ToolCallStarted { .. },\n}',
    status: 'success' as const,
    elapsedMs: 12,
  },
  {
    toolName: 'edit',
    args: {
      path: 'app/src/App.tsx',
      old_string: 'const theme = "light";',
      new_string: 'const theme = useTheme();\nconst accent = theme.accent;',
    },
    result: 'ok',
    status: 'success' as const,
    elapsedMs: 40,
  },
  {
    toolName: 'shell',
    args: { command: 'pnpm test --run tools' },
    result:
      ' ✓ toolPresentation.test.ts (22)\n ✓ parseWebSearchResult.test.ts (8)\n\n Test Files  2 passed',
    status: 'success' as const,
    elapsedMs: 5230,
  },
  {
    toolName: 'web_fetch',
    args: { url: 'https://docs.rs/tokio/latest/tokio/' },
    result:
      'status=200 url=https://docs.rs/tokio/latest/tokio/ content=markdown\n# Tokio\n\nA runtime for writing **reliable** asynchronous applications with Rust.',
    status: 'success' as const,
    elapsedMs: 620,
  },
  {
    toolName: 'GMAIL_SEND_EMAIL',
    args: { to: 'alex@example.com', subject: 'Q3 plan' },
    result: '{"successful":true}',
    status: 'success' as const,
    elapsedMs: 910,
  },
  {
    toolName: 'mcp_call_tool',
    args: { server: 'linear', tool: 'create_issue', arguments: { title: 'Fix labels' } },
    status: 'running' as const,
  },
  {
    toolName: 'memory',
    args: { action: 'recall', query: 'preferred meeting times' },
    result: 'Mornings before 11am.',
    status: 'success' as const,
    elapsedMs: 88,
  },
  {
    toolName: 'grep',
    args: { pattern: 'display_label' },
    status: 'error' as const,
    result: 'regex parse error',
    failure: {
      class: 'InvalidInput',
      category: 'Recoverable',
      recoverable: true,
      causePlain: 'The search pattern was not a valid regular expression.',
      nextAction: 'The agent will retry with an escaped pattern.',
    },
  },
  { toolName: 'cron', args: { action: 'add', name: 'Daily digest' }, status: 'cancelled' as const },
  { toolName: 'some_new_tool', args: { name: 'widget' }, status: 'success' as const, result: 'ok' },
];

function CatalogRow({ name }: { name: string }) {
  const { t } = useT();
  const running = describeToolCall({ name, status: 'running' });
  const done = describeToolCall({ name, status: 'success' });
  return (
    <li className="flex items-center gap-2 py-1 text-xs" data-testid="tool-gallery-catalog-row">
      <ToolIcon presentation={done} className="text-foreground/50 size-3.5" />
      <span className="text-foreground/40 w-56 shrink-0 truncate font-mono">{name}</span>
      <span className="text-foreground/80 w-56 shrink-0 truncate">{toolLabel(running, t)}</span>
      <span className="text-foreground/60 truncate">{toolLabel(done, t)}</span>
    </li>
  );
}

export default function ToolCallGallery() {
  const { t } = useT();
  const [streaming, setStreaming] = useState(true);
  const [scheduleEnabled, setScheduleEnabled] = useState(true);
  const [searchQuery, setSearchQuery] = useState('deploy');
  const [searchActive, setSearchActive] = useState(0);
  const [citationOpen, setCitationOpen] = useState<number | null>(null);
  const [connectionPhase, setConnectionPhase] = useState<(typeof MOCK_CONNECTION_PHASES)[number]>(
    MOCK_CONNECTION_PHASES[0]
  );
  return (
    <div className="bg-background text-foreground min-h-screen overflow-auto p-8">
      <div className="mx-auto flex max-w-3xl flex-col gap-10">
        <header>
          <h1 className="text-lg font-semibold">Tool calls</h1>
          <p className="text-foreground/50 text-sm">
            assistant-ui tool-call, tool-timeline and web-search elements over the presentation
            registry.
          </p>
        </header>

        <section className="flex flex-col gap-3">
          <label className="text-foreground/60 flex items-center gap-2 text-xs">
            <input
              type="checkbox"
              checked={streaming}
              onChange={event => setStreaming(event.target.checked)}
            />
            Timeline streaming
          </label>
          <ToolTimeline
            className="max-w-none"
            defaultOpen
            streaming={streaming}
            activeLabel={toolLabel(
              describeToolCall({ name: 'web_search_tool', status: 'running' }),
              t
            )}
            restingLabel="4 steps · Searched the web, Read file, Edited file, Ran command">
            {SAMPLES.slice(0, 5).map((sample, index) => (
              <AssistantUiToolCallCard key={index} {...sample} />
            ))}
          </ToolTimeline>
        </section>

        <section className="flex flex-col gap-1">
          <h2 className="text-foreground/60 mb-2 text-xs font-medium uppercase">Every state</h2>
          {SAMPLES.map((sample, index) => (
            <AssistantUiToolCallCard key={index} {...sample} />
          ))}
          <AssistantUiToolCallCard
            toolName="composio_execute"
            args={{ tool: 'SLACK_SEND_MESSAGE' }}
            awaitingUser
            footer={<p className="text-foreground/50 ps-5 text-xs">(approval card renders here)</p>}
          />
        </section>

        <section className="flex flex-col gap-3">
          <h2 className="text-foreground/60 mb-2 text-xs font-medium uppercase">Approvals</h2>

          <p className="text-foreground/40 text-xs">Pending, in-thread (chat-approval-*)</p>
          <ApprovalCardAdapter
            ariaLabel="Approval needed"
            title="Approval needed"
            subtitle={APPROVAL_PENDING_APPROVAL.message}
            command={APPROVAL_PENDING_APPROVAL.command ?? ''}
            toolName={APPROVAL_PENDING_APPROVAL.toolName}
            alwaysDecision="approve_always_for_tool"
            analyticsPrefix="chat-approval"
            onDecide={async () => {}}
          />

          <p className="text-foreground/40 text-xs">Pending with a live expiry countdown</p>
          <ApprovalCardAdapter
            ariaLabel="Approval needed"
            title="Approval needed"
            subtitle={APPROVAL_EXPIRING.message}
            command={APPROVAL_EXPIRING.command ?? ''}
            toolName={APPROVAL_EXPIRING.toolName}
            expiresAt={APPROVAL_EXPIRING.expiresAt}
            alwaysDecision="approve_always_for_tool"
            analyticsPrefix="chat-approval"
            onDecide={async () => {}}
          />

          <p className="text-foreground/40 text-xs">Denied (no always-allow, unrouted surface)</p>
          <ApprovalCardAdapter
            ariaLabel="Approval needed"
            title="Approval needed"
            subtitle="Background task needs approval"
            command="triage.escalate"
            toolName="triage.escalate"
            analyticsPrefix="unrouted-approval"
            onDecide={() => Promise.reject(new Error('rejected for the gallery'))}
          />

          <p className="text-foreground/40 text-xs">
            composio_connect (permission-grant, one Connect action)
          </p>
          <PermissionGrantAdapter threadId="dev-thread" approval={COMPOSIO_CONNECT_APPROVAL} />

          <p className="text-foreground/40 text-xs">Elicitation — ask_user_clarification</p>
          <ElicitationAdapter
            server="OpenHuman"
            message="Which repository should I open a PR against?"
            pending
            onAnswer={() => {}}
            testId="tool-gallery-elicitation"
          />
          <ElicitationAdapter
            server="OpenHuman"
            message="Which repository should I open a PR against?"
            pending={false}
            onAnswer={() => {}}
          />
        </section>

        <section className="flex flex-col gap-3">
          <h2 className="text-foreground/60 mb-2 text-xs font-medium uppercase">
            Goals, todos, plan review
          </h2>

          <p className="text-foreground/40 text-xs">
            Goal — pinned `AgentStatus` pill (mapped from `thread_goal_updated`)
          </p>
          <AgentStatus
            state="working"
            label="Cover every element the demo transcript can render"
            trailing={<span className="tabular-nums">1.2k / 20k</span>}
          />

          <p className="text-foreground/40 text-xs">
            Todos — pinned `TodoList` (mapped from `thread_todos_changed`)
          </p>
          <TodoList
            title="Todos"
            items={[
              { id: '0', text: 'Stream reasoning and prose', status: 'done' },
              { id: '1', text: 'Run a tool call and a delegation', status: 'done' },
              { id: '2', text: 'Render the goal and plan-review elements', status: 'active' },
              { id: '3', text: 'Wrap up with the closing summary', status: 'pending' },
            ]}
          />

          <p className="text-foreground/40 text-xs">
            Plan review — pending decision (`request_plan_review`, `AgentPlan` + approve / reject /
            revise)
          </p>
          <PlanReviewCardCore
            threadId="dev-thread"
            review={{
              requestId: 'dev-plan-review',
              summary: 'Render the goal, todo and plan-review elements for this gallery',
              steps: [
                'Render the goal and todo elements inline',
                'Show the plan under review',
                'Resolve the review and continue',
              ],
            }}
          />

          <p className="text-foreground/40 text-xs">
            Plan review — already decided / replayed history (`activeIndex: steps.length`)
          </p>
          <AgentPlan
            title="Review plan"
            steps={['Render the goal and todo elements inline', 'Show the plan under review']}
            activeIndex={2}
          />
        </section>

        <section className="flex flex-col gap-1">
          <h2 className="text-foreground/60 mb-2 text-xs font-medium uppercase">Message queue</h2>
          <MessageQueue
            data-testid="tool-gallery-message-queue"
            running={MOCK_MESSAGE_QUEUE.running}
            queued={MOCK_MESSAGE_QUEUE.queued}
            onCancel={() => {}}
            runningLabel={t('chat.messageQueue.running')}
            queuedLabel={count =>
              t('chat.messageQueue.queuedCount').replace('{count}', String(count))
            }
            pendingHint={t('chat.messageQueue.pendingHint')}
            removeLabel={text => t('chat.messageQueue.remove').replace('{text}', text)}
          />
        </section>

        <section className="flex flex-col gap-2" data-testid="tool-gallery-followup-suggestions">
          <h2 className="text-foreground/60 mb-2 text-xs font-medium uppercase">
            Follow-up suggestions (chat_suggestions)
          </h2>
          <FollowupSuggestionsDemo />
        </section>

        <section className="flex flex-col gap-2">
          <h2 className="text-foreground/60 mb-2 text-xs font-medium uppercase">
            Connection state
          </h2>
          <div className="text-foreground/60 flex gap-3 text-xs">
            {MOCK_CONNECTION_PHASES.map(phase => (
              <label key={phase} className="flex items-center gap-1">
                <input
                  type="radio"
                  name="connection-phase"
                  checked={connectionPhase === phase}
                  onChange={() => setConnectionPhase(phase)}
                />
                {phase}
              </label>
            ))}
          </div>
          <ConnectionStateNotice
            phase={connectionPhase}
            onRetry={() => setConnectionPhase('reconnecting')}
          />
        </section>

        <section className="flex flex-col gap-2">
          <h2 className="text-foreground/60 mb-2 text-xs font-medium uppercase">
            Context usage (ring + breakdown popover body)
          </h2>
          <ContextDisplayRing
            data-testid="tool-gallery-context-ring"
            aria-label={t('conversations.composer.context.usage')}
            modelContextWindow={MOCK_CONTEXT_USAGE.modelContextWindow}
            usage={MOCK_CONTEXT_USAGE.usage}
            className="self-start"
          />
          <ContextBreakdown
            data-testid="tool-gallery-context-breakdown"
            segments={contextBreakdownSegments(MOCK_CONTEXT_BREAKDOWN, t)}
            limit={MOCK_CONTEXT_BREAKDOWN.context_window}
            title={t('conversations.composer.context.title')}
            headroomLabel={t('conversations.composer.context.headroom')}
          />
        </section>

        <section className="flex flex-col gap-2">
          <h2 className="text-foreground/60 mb-2 text-xs font-medium uppercase">
            Chat settings (settings-panel, read-only)
          </h2>
          {/* What ChatSettingsPanel renders: model + temperature only — no core
              config RPC stores a chat system prompt, so that field is omitted. */}
          <SettingsPanel
            data-testid="tool-gallery-settings-panel"
            model="gpt-4o-mini"
            models={['OpenHuman', 'gpt-4o-mini']}
            temperature={0.7}
            modelLabel={t('composer.settings.model')}
            temperatureLabel={t('composer.settings.temperature')}
            temperatureAriaLabel={t('composer.settings.temperature')}
          />
        </section>

        <section className="flex flex-col gap-3">
          <h2 className="text-foreground/60 mb-2 text-xs font-medium uppercase">
            Rich content &amp; conversation map (WS-G)
          </h2>

          <p className="text-foreground/40 text-xs">Sources — url + document (memory citation)</p>
          <div className="flex flex-wrap items-center gap-1.5">
            <Source href="https://docs.rs/tokio/latest/tokio/">
              <SourceIcon url="https://docs.rs/tokio/latest/tokio/" />
              <SourceTitle>docs.rs</SourceTitle>
            </Source>
            <Source href="https://blog.rust-lang.org/2023/12/21/async-fn-rpit-in-traits.html">
              <SourceIcon url="https://blog.rust-lang.org/2023/12/21/async-fn-rpit-in-traits.html" />
              <SourceTitle>blog.rust-lang.org</SourceTitle>
            </Source>
          </div>

          <p className="text-foreground/40 text-xs">
            Inline citation marker (hover for the source)
          </p>
          <p className="text-foreground/80 text-sm">
            The deploy runs nightly
            <CitationMarker
              index={0}
              source={{
                domain: 'docs.rs',
                title: 'tokio scheduler docs',
                snippet: 'The default runtime schedules a nightly compaction pass.',
              }}
              open={citationOpen === 0}
              onOpenChange={open => setCitationOpen(open ? 0 : null)}
            />
            .
          </p>

          <p className="text-foreground/40 text-xs">Memory chips (stored this turn + existing)</p>
          <MemoryChips
            chips={[
              { id: 'm1', text: 'preferred_meeting_time', change: 'added' },
              { id: 'm2', text: 'timezone', change: 'existing' },
            ]}
            onForget={() => {}}
          />

          <p className="text-foreground/40 text-xs">Schedule card (cron_add / cron_update)</p>
          <ScheduleCard
            name="Daily digest"
            cadence="0 9 * * *"
            nextRun="2026-01-02T09:00:00.000Z"
            enabled={scheduleEnabled}
            history={[
              { id: 'run-1', at: '2026-01-01T09:00:00.000Z', ok: true },
              { id: 'run-2', at: '2025-12-31T09:00:00.000Z', ok: false },
            ]}
            onToggle={() => setScheduleEnabled(enabled => !enabled)}
          />

          <p className="text-foreground/40 text-xs">Conversation search (find-in-conversation)</p>
          <ConversationSearch
            query={searchQuery}
            hits={MEMORY_SEARCH_HITS}
            activeIndex={searchActive}
            onQueryChange={setSearchQuery}
            onStep={delta =>
              setSearchActive(
                index => (index + delta + MEMORY_SEARCH_HITS.length) % MEMORY_SEARCH_HITS.length
              )
            }
          />

          <p className="text-foreground/40 text-xs">Timeline (conversation map outline)</p>
          <Timeline events={MEMORY_TIMELINE_EVENTS} visibleCount={MEMORY_TIMELINE_EVENTS.length} />
        </section>

        <section className="flex flex-col gap-2">
          <h2 className="text-foreground/60 mb-2 text-xs font-medium uppercase">
            Composer / and @ menus
          </h2>
          <ComposerMenu
            open
            data-testid="tool-gallery-slash-menu"
            className="relative bottom-auto mb-0">
            {MOCK_COMMANDS_LIST.map((command, index) => (
              <ComposerCommandItem
                key={command.id}
                active={index === 0}
                command={{
                  name: command.id,
                  description: command.description ?? command.label,
                  icon: COMMAND_KIND_ICONS[command.kind],
                }}
              />
            ))}
          </ComposerMenu>
          <ComposerMenu
            open
            data-testid="tool-gallery-mention-menu"
            className="relative bottom-auto mb-0">
            {MOCK_MEMORY_FETCH.hits.map((hit, index) => (
              <ComposerMenuItem key={hit.id} active={index === 0}>
                <BrainIcon className="text-foreground/35 size-3.5 shrink-0" />
                <span className="flex-1 truncate text-start">{hit.text}</span>
              </ComposerMenuItem>
            ))}
            {MOCK_THREAD_FILES.map(file => (
              <ComposerMenuItem key={file.id}>
                <FileIcon className="text-foreground/35 size-3.5 shrink-0" />
                <span className="flex-1 truncate text-start">{file.label}</span>
              </ComposerMenuItem>
            ))}
          </ComposerMenu>
        </section>

        <section>
          <h2 className="text-foreground/60 mb-2 text-xs font-medium uppercase">
            Core catalog ({(coreToolNames as string[]).length})
          </h2>
          <ul className="divide-foreground/[0.06] divide-y">
            {(coreToolNames as string[]).map(name => (
              <CatalogRow key={name} name={name} />
            ))}
          </ul>
        </section>
      </div>
    </div>
  );
}
