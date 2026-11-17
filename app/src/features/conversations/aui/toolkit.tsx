import {
  defineToolkit,
  type ToolCallMessagePartComponent,
  type Toolkit,
} from '@assistant-ui/react';
import { useMemo } from 'react';

import {
  MemoryHybridSearchCall,
  MemoryRecallCall,
  MemoryStoreCall,
  MemoryToolCall,
} from './ChatMemoryChips';
import { CronAddOrUpdateCall, CronListCall, CronRunsCall } from './ChatScheduleCard';
import { GoalToolLine } from './GoalToolLine';
import { DocumentArtifactCall, MediaGenerationCall } from './MediaAndDocumentCalls';
import { PlanReviewPart } from './PlanReviewPart';
import { SubagentTaskCard } from './SubagentTaskCard';
import { TodoListPart } from './TodoListPart';

/**
 * One assistant-ui toolkit entry.
 *
 * Every OpenHuman tool the model can call is executed by the core, never the
 * browser, so every entry is `type: 'backend'` — assistant-ui never tries to
 * run it and never expects `description`/`parameters` from us (those are
 * owned by the core's tool schema, sent to the model over the wire). The only
 * thing an entry contributes on the frontend is *how a call renders*.
 *
 * `display` follows assistant-ui's chain-of-thought convention: `'inline'`
 * (the default) folds the call into the same activity trace as every other
 * tool call; `'standalone'` pulls it out for something that deserves its own
 * spot in the transcript (a generated image, a produced document).
 */
export interface OpenHumanToolEntry {
  type: 'backend';
  display?: 'inline' | 'standalone';
  render: ToolCallMessagePartComponent;
}

/**
 * The toolkit registry, keyed by the tool name the core sends on the wire.
 *
 * A tool name NOT listed here is not an error: assistant-ui falls through to
 * the surface's own `components.ToolFallback` (`ChatToolFallback` in
 * `ChatToolParts.tsx`), which is how every ordinary/dynamic tool (shell, file
 * ops, MCP, Composio, web search, ...) has always rendered and still does —
 * including the approval-gate and `composio_connect` routing, both orthogonal
 * to any one tool's name and therefore not something a per-name registry can
 * own. Only tools whose call deserves its *own* rich element belong here.
 *
 * A function, not a module-level object: `ChatToolParts.tsx` imports from
 * `AssistantUiRuntimeProvider.tsx` (for `useAuiThreadId`), which imports this
 * module (for the toolkit) — a real cycle. Evaluating `SubagentCall` in a
 * module-scope object literal races that cycle: whichever side of it loads
 * first can capture `SubagentCall` before `ChatToolParts.tsx` has finished
 * defining it, baking `undefined` into a frozen entry. Building the record
 * inside a function defers that read to call time, after every module in the
 * cycle has finished loading.
 *
 * To add an entry: import the render component and add a key here. Nothing
 * else in this module needs to change — `buildOpenHumanToolkit` /
 * `useOpenHumanToolkit` pick up every entry automatically.
 */
export function openHumanToolEntries(): Record<string, OpenHumanToolEntry> {
  return {
    /**
     * A sub-agent delegation. Never approval-gated (the orchestrator spawns
     * it directly), so its render skips the gate check every other entry
     * would need and goes straight to `SubagentTaskCard`, over the vendored
     * `task-card` element — nested `messages` transcript and the awaiting-user
     * reply box all live there now (`AssistantUiSubagentCall`
     * / `SubagentDrawer` are deleted).
     */
    task: { type: 'backend', display: 'standalone', render: SubagentTaskCard },

    /**
     * Image / video generation: the `elements-image-generation` placeholder
     * while it runs, then the `image` element per produced artifact. Pulled
     * out for its own spot in the transcript rather than folded into the
     * activity trace, same as a produced document below.
     */
    media_generate_image: { type: 'backend', display: 'standalone', render: MediaGenerationCall },
    media_generate_video: { type: 'backend', display: 'standalone', render: MediaGenerationCall },

    /**
     * `generate_document` / `generate_presentation`: the `elements-artifact-
     * card` element.
     */
    generate_document: { type: 'backend', display: 'standalone', render: DocumentArtifactCall },
    generate_presentation: { type: 'backend', display: 'standalone', render: DocumentArtifactCall },

    /**
     * Memory writes/reads, rendered as `memory-chips` instead of the raw
     * JSON `ToolDataView` fallback (`ChatMemoryChips.tsx`).
     */
    memory: { type: 'backend', display: 'inline', render: MemoryToolCall },
    memory_store: { type: 'backend', display: 'inline', render: MemoryStoreCall },
    memory_recall: { type: 'backend', display: 'inline', render: MemoryRecallCall },
    memory_hybrid_search: { type: 'backend', display: 'inline', render: MemoryHybridSearchCall },

    /**
     * Cron reads/writes, rendered as `schedule-card` (`ChatScheduleCard.tsx`).
     */
    cron_add: { type: 'backend', display: 'inline', render: CronAddOrUpdateCall },
    cron_update: { type: 'backend', display: 'inline', render: CronAddOrUpdateCall },
    cron_list: { type: 'backend', display: 'inline', render: CronListCall },
    cron_runs: { type: 'backend', display: 'inline', render: CronRunsCall },

    /**
     * The agent's whole-list todo write (Claude Code / Codex style), rendered
     * as the vendored `TodoList` element per call (`TodoListPart.tsx`). The
     * always-current, pinned todo list above the composer is a SEPARATE
     * render driven by the live `thread_todos_changed` socket event
     * (`useThreadTodos`), not this per-call snapshot.
     */
    todo: { type: 'backend', display: 'standalone', render: TodoListPart },

    /**
     * The durable per-thread goal tools, rendered as a compact one-line
     * summary in the activity trace (`GoalToolLine.tsx`). The pinned goal
     * pill above the composer is a separate render driven by
     * `thread_goal_updated` / `thread_goal_cleared` (`useThreadGoal`).
     */
    goal_set: { type: 'backend', display: 'inline', render: GoalToolLine },
    goal_get: { type: 'backend', display: 'inline', render: GoalToolLine },
    goal_complete: { type: 'backend', display: 'inline', render: GoalToolLine },

    /**
     * Plan-mode review gate: the orchestrator parked the live turn on a
     * thread-scoped plan (`request_plan_review`). Rendered as the vendored
     * `AgentPlan` element plus an approve/reject/revise decision row while
     * the review is still pending (`PlanReviewPart.tsx`).
     */
    request_plan_review: { type: 'backend', display: 'standalone', render: PlanReviewPart },
  };
}

/**
 * Build the toolkit. `defineToolkit` only types/validates the entries; the
 * object it returns is cheap to recompute, so callers that are not React
 * components (tests, non-hook call sites) can call this directly instead of
 * the hook.
 */
export function buildOpenHumanToolkit(): Toolkit {
  return defineToolkit(openHumanToolEntries());
}

/**
 * The toolkit for the runtime provider's `config` (`AuiConfig({ tools: Tools({
 * toolkit }) })` in {@link AssistantUiRuntimeProvider}). Entries are static —
 * not derived from props or Redux — so the memo never recomputes after the
 * first render.
 */
export function useOpenHumanToolkit(): Toolkit {
  return useMemo(() => buildOpenHumanToolkit(), []);
}
