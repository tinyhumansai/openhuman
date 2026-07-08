/**
 * SessionTranscript — renders an orchestration session/chat transcript in the
 * app's normal chat-window visual language (right-aligned solid-primary user
 * bubbles, left neutral agent bubbles), with v2 harness activity woven in as
 * inline non-bubble blocks: merged tool call+result (red on failure), thinking,
 * error, and approval_request. Shared by the Agent chat pane and the Connections
 * session view so a session reads identically wherever it appears.
 *
 * Approvals are actionable only on your OWN agent (master/subconscious) — pass
 * `onDecide`. In a peer session omit it and the approval renders read-only.
 */
import type { ReactElement } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { mergeToolActivity, type ToolActivity } from '../../lib/orchestration/mergeToolActivity';
import type { ChatMessage } from '../../lib/orchestration/useOrchestrationChats';
import { formatTime } from '../intelligence/orchestrationTabHelpers';

export type ApprovalDecision = 'approve' | 'deny' | 'always';

export interface SessionTranscriptProps {
  messages: ChatMessage[];
  /** When present, approval rows show actionable buttons wired to this. */
  onDecide?: (message: ChatMessage, decision: ApprovalDecision) => void;
}

/**
 * Whether a row's `from` marks it as owner/user-authored (right-side bubble).
 * The composer's own reply is mirrored back with role `"owner"`; the master
 * optimistic append uses the localized "you". Both belong on the right.
 */
function isOwnerAuthored(from: string): boolean {
  return from === 'you' || from === 'owner' || from === 'user';
}

/** Lightweight `**bold**` rendering without pulling in a markdown lib. */
function renderInline(text: string): (string | ReactElement)[] {
  return text.split(/(\*\*[^*]+\*\*)/g).map((part, i) =>
    part.startsWith('**') && part.endsWith('**') ? (
      <strong key={i} className="font-semibold">
        {part.slice(2, -2)}
      </strong>
    ) : (
      part
    )
  );
}

function UserBubble({ message }: { message: ChatMessage }): ReactElement {
  return (
    <div className="flex justify-end" data-event-kind="user_prompt">
      <div className="flex max-w-[75%] flex-col items-end gap-1">
        <div className="overflow-hidden break-words rounded-2xl rounded-br-md bg-primary-500 px-4 py-2.5 text-content-inverted">
          <p className="whitespace-pre-wrap text-sm leading-relaxed">{message.body}</p>
        </div>
        <span className="px-1 text-[10px] text-white/60">{formatTime(message.timestamp)}</span>
      </div>
    </div>
  );
}

function AgentBubble({ message }: { message: ChatMessage }): ReactElement {
  return (
    <div className="flex justify-start" data-event-kind={message.eventKind ?? 'v1'}>
      <div className="flex max-w-[75%] flex-col items-start gap-1">
        <div className="rounded-2xl rounded-bl-md bg-surface-strong px-4 py-2.5 text-content dark:bg-surface-muted/80">
          <p className="whitespace-pre-wrap text-sm leading-relaxed">
            {renderInline(message.body)}
          </p>
        </div>
        <span className="px-1 text-[10px] text-content-faint">{formatTime(message.timestamp)}</span>
      </div>
    </div>
  );
}

function ThinkingRow({ message }: { message: ChatMessage }): ReactElement {
  return (
    <div className="flex justify-start" data-event-kind="agent_thinking">
      <div className="flex max-w-[75%] items-start gap-2 px-1 text-content-faint">
        <span className="mt-0.5 flex-none text-xs">∴</span>
        <p className="whitespace-pre-wrap text-xs italic leading-relaxed">{message.body}</p>
      </div>
    </div>
  );
}

function ErrorRow({ message }: { message: ChatMessage }): ReactElement {
  return (
    <div className="flex justify-start" data-event-kind="error">
      <div className="flex w-full max-w-[85%] items-start gap-2 rounded-xl border border-coral-200 bg-coral-50 px-3 py-2 dark:border-coral-500/30 dark:bg-coral-500/10">
        <span className="mt-0.5 flex-none text-xs text-coral-500">✕</span>
        <p className="whitespace-pre-wrap text-xs leading-relaxed text-coral-700 dark:text-coral-300">
          {message.body}
        </p>
      </div>
    </div>
  );
}

function ToolBlock({ tool }: { tool: ToolActivity }): ReactElement {
  return (
    <div className="flex justify-start" data-event-kind="tool_call" data-failed={tool.failed}>
      <div className="w-full max-w-[85%] overflow-hidden rounded-xl border border-line bg-surface-subtle">
        <div className="flex items-center gap-2 border-b border-line-subtle px-3 py-1.5">
          <span className="flex-none text-xs text-content-faint">▶</span>
          {tool.toolName ? (
            <span className="flex-none rounded bg-surface-strong px-1.5 py-0.5 font-mono text-[10px] font-medium text-content-secondary">
              {tool.toolName}
            </span>
          ) : null}
          <code className="min-w-0 flex-1 truncate font-mono text-[11px] text-content-muted">
            {tool.command}
          </code>
        </div>
        {tool.hasResult ? (
          <div
            className={`flex gap-2 px-3 py-2 ${tool.failed ? 'bg-coral-50 dark:bg-coral-500/10' : ''}`}>
            <span
              className={`mt-0.5 flex-none text-xs ${tool.failed ? 'text-coral-500' : 'text-sage-500'}`}
              aria-hidden>
              {tool.failed ? '✕' : '↳'}
            </span>
            <pre
              className={`min-w-0 flex-1 overflow-x-auto whitespace-pre-wrap font-mono text-[11px] leading-relaxed ${
                tool.failed ? 'text-coral-700 dark:text-coral-300' : 'text-content-muted'
              }`}>
              {tool.output}
            </pre>
          </div>
        ) : null}
      </div>
    </div>
  );
}

function ApprovalRow({
  message,
  onDecide,
}: {
  message: ChatMessage;
  onDecide?: (message: ChatMessage, decision: ApprovalDecision) => void;
}): ReactElement {
  const { t } = useT();
  return (
    <div className="flex justify-start" data-event-kind="approval_request">
      <div className="w-full max-w-[85%] rounded-xl border border-amber-300 bg-amber-50 px-3 py-2.5 dark:border-amber-500/40 dark:bg-amber-500/10">
        <div className="flex items-center gap-2">
          <span className="text-sm text-amber-500">⚠</span>
          <span className="text-xs font-semibold text-amber-700 dark:text-amber-300">
            {t('chat.approval.title')}
          </span>
          {message.toolName ? (
            <span className="rounded bg-amber-100 px-1.5 py-0.5 font-mono text-[10px] text-amber-700 dark:bg-amber-500/20 dark:text-amber-200">
              {message.toolName}
            </span>
          ) : null}
        </div>
        <code className="mt-1.5 block overflow-x-auto whitespace-pre-wrap break-words font-mono text-[11px] text-content-secondary">
          {message.body}
        </code>
        {onDecide ? (
          <div className="mt-2.5 flex gap-2">
            <button
              type="button"
              onClick={() => onDecide(message, 'approve')}
              className="rounded-lg bg-amber-500 px-3 py-1 text-xs font-semibold text-white transition hover:bg-amber-600">
              {t('chat.approval.approve')}
            </button>
            <button
              type="button"
              onClick={() => onDecide(message, 'deny')}
              className="rounded-lg border border-line bg-surface px-3 py-1 text-xs font-medium text-content-secondary transition hover:bg-surface-hover">
              {t('chat.approval.deny')}
            </button>
            <button
              type="button"
              onClick={() => onDecide(message, 'always')}
              className="rounded-lg px-2 py-1 text-xs font-medium text-content-faint transition hover:text-content-secondary">
              {t('chat.approval.alwaysAllow')}
            </button>
          </div>
        ) : null}
      </div>
    </div>
  );
}

export default function SessionTranscript({
  messages,
  onDecide,
}: SessionTranscriptProps): ReactElement {
  const rows = mergeToolActivity(messages);
  return (
    <div className="space-y-3" data-testid="session-transcript">
      {rows.map((row, i) => {
        if (row.kind === 'tool') return <ToolBlock key={row.id} tool={row} />;
        const { message } = row;
        switch (message.eventKind) {
          case 'user_prompt':
            return <UserBubble key={message.id} message={message} />;
          case 'agent_thinking':
            return <ThinkingRow key={message.id} message={message} />;
          case 'error':
            return <ErrorRow key={message.id} message={message} />;
          case 'approval_request':
            return <ApprovalRow key={message.id} message={message} onDecide={onDecide} />;
          default:
            // agent_message + legacy v1 rows → bubble by sender. Owner/user-
            // authored rows (incl. a reply mirrored back with role "owner") sit
            // on the right; everything else is an agent bubble on the left.
            return isOwnerAuthored(message.from) ? (
              <UserBubble key={message.id} message={message} />
            ) : (
              <AgentBubble key={`${message.id}-${i}`} message={message} />
            );
        }
      })}
    </div>
  );
}
