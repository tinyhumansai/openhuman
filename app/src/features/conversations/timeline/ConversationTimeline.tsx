import type { SubagentActivity, ToolTimelineEntry } from '../../../store/chatRuntimeSlice';
import { ToolTimelineBlock } from '../components/ToolTimelineBlock';
import { AssistantMessageItem } from './items/AssistantMessageItem';
import { StreamingTailItem } from './items/StreamingTailItem';
import { UserMessageItem } from './items/UserMessageItem';
import type { TimelineItem } from './types';

export interface ConversationTimelineHandlers {
  /** Opens the full-transcript drawer for a subagent row. */
  onOpenSubagent?: (subagent: SubagentActivity) => void;
  /** Opens the whole-run "Agent Process Source" panel. */
  onViewWholeRun?: () => void;
}

/** True for the process kinds that coalesce into one `ToolTimelineBlock`. */
function isProcessItem(item: TimelineItem): item is TimelineItem & { entry: ToolTimelineEntry } {
  return item.kind === 'toolCall' || item.kind === 'subagentActivity';
}

/**
 * Pure renderer: `TimelineItem[]` → one element per kind, wrapping the existing
 * components (see `docs/plans/conversations-timeline-refactor.md` Phase 2).
 *
 * Consecutive process items (`toolCall`/`subagentActivity`) are coalesced into a
 * single `ToolTimelineBlock` so the timeline reads as one grouped block,
 * matching today's `agentInsights` anchor. Ordering, anchoring, and
 * `hideAgentInsights` filtering live upstream in the selector — this component
 * only maps items to UI.
 */
export function ConversationTimeline({
  items,
  agentMessageViewMode = 'bubbles',
  handlers = {},
}: {
  items: TimelineItem[];
  agentMessageViewMode?: 'text' | 'bubbles';
  handlers?: ConversationTimelineHandlers;
}) {
  const rendered: React.ReactNode[] = [];

  for (let i = 0; i < items.length; i += 1) {
    const item = items[i];

    // Coalesce a contiguous run of process items into one ToolTimelineBlock.
    if (isProcessItem(item)) {
      const run: ToolTimelineEntry[] = [];
      const startId = item.id;
      let j = i;
      while (j < items.length && isProcessItem(items[j])) {
        run.push((items[j] as TimelineItem & { entry: ToolTimelineEntry }).entry);
        j += 1;
      }
      rendered.push(
        <ToolTimelineBlock
          key={`process:${startId}`}
          entries={run}
          onViewSubagent={handlers.onOpenSubagent}
          onViewWholeRun={handlers.onViewWholeRun}
        />
      );
      i = j - 1;
      continue;
    }

    switch (item.kind) {
      case 'userMessage':
        rendered.push(<UserMessageItem key={item.id} content={item.message.content} />);
        break;
      case 'assistantMessage':
        rendered.push(
          <AssistantMessageItem
            key={item.id}
            content={item.message.content}
            viewMode={agentMessageViewMode}
          />
        );
        break;
      case 'streamingText':
        rendered.push(
          <StreamingTailItem
            key={item.id}
            text={item.text}
            thinking={item.thinking}
            branch={item.branch}
          />
        );
        break;
      case 'reasoning':
        // Reasoning is rendered inside the process/streaming affordances today;
        // no standalone element yet.
        break;
      default:
        break;
    }
  }

  return (
    <div data-testid="conversation-timeline" className="space-y-3">
      {rendered}
    </div>
  );
}
