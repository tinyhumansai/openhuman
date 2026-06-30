/**
 * TeamActivityRail — the live teammate-message timeline for an agent team
 * (#3374 PR2).
 *
 * This is the surface's differentiator: a board of owned tasks is commodity,
 * but seeing the agents *talk to each other while they work* is the thing that
 * makes multi-agent coordination legible. Each entry is a `team_message` run
 * event resolved to `from → to` (member names, with the member's deterministic
 * colour) plus the message body. A `null` recipient means the message was sent
 * to the whole team.
 *
 * The rail renders inside a grid cell that collapses *under* the board on
 * narrow widths (handled by the parent tab's responsive grid), so it is always
 * visible on a wide Intelligence pane and stacks gracefully when cramped.
 */
import { useState } from 'react';
import { LuSend } from 'react-icons/lu';

import { useT } from '../../lib/i18n/I18nContext';
import type { AgentTeamMember, TeamMessage } from '../../services/api/agentTeamApi';
import { memberColor } from './memberColors';

interface TeamActivityRailProps {
  messages: TeamMessage[];
  members: AgentTeamMember[];
  /**
   * When provided, renders a composer footer so the user (as the lead) can
   * address a named teammate, or the whole team when `toMemberId` is `null`.
   */
  onSend?: (toMemberId: string | null, content: string) => void | Promise<void>;
  /** True while a send is in flight (disables the composer). */
  sending?: boolean;
}

export function TeamActivityRail({ messages, members, onSend, sending }: TeamActivityRailProps) {
  const { t } = useT();
  const memberById = new Map(members.map(m => [m.id, m]));

  const [draft, setDraft] = useState('');
  const [recipient, setRecipient] = useState<string>('');

  const submit = () => {
    const content = draft.trim();
    if (!content || sending || !onSend) return;
    // Clear the draft only on a resolved send; on rejection keep the unsent text
    // so the user can retry. The empty .catch() handles the rejection (the parent
    // surfaces the error via its own notice state) and prevents an unhandled
    // promise rejection from the now-throwing onSend.
    void Promise.resolve(onSend(recipient === '' ? null : recipient, content))
      .then(() => {
        setDraft('');
      })
      .catch(() => {
        /* send failed — draft retained; error surfaced by the parent */
      });
  };

  const nameFor = (id: string | null): string => {
    if (!id) return t('intelligence.teams.activity.toTeam');
    return memberById.get(id)?.name ?? id;
  };

  return (
    <aside className="rounded-lg border border-line bg-surface p-3">
      <div className="mb-2 flex items-center justify-between">
        <h3 className="text-[11px] font-semibold uppercase tracking-wide text-content-muted">
          {t('intelligence.teams.activity.title')}
        </h3>
        <span className="text-[10px] text-content-faint">{messages.length}</span>
      </div>

      {messages.length === 0 ? (
        <p className="py-6 text-center text-[11px] text-content-faint">
          {t('intelligence.teams.activity.empty')}
        </p>
      ) : (
        <div className="space-y-3">
          {messages.map(message => {
            const fromMember = memberById.get(message.payload.from);
            const fromName = fromMember?.name ?? message.payload.from;
            const color = memberColor(message.payload.from);
            return (
              <div key={`${message.runId}-${message.sequence}`} className="flex gap-2">
                <span
                  className="mt-0.5 inline-flex h-5 w-5 flex-none items-center justify-center rounded-full text-[9px] font-semibold text-white"
                  style={{ backgroundColor: color }}>
                  {fromName.charAt(0).toUpperCase()}
                </span>
                <div className="min-w-0">
                  <div className="text-[10px] text-content-faint">
                    <b className="text-content-secondary">{fromName}</b>
                    {' → '}
                    {nameFor(message.payload.to)}
                  </div>
                  <p className="break-words text-[11px] leading-snug text-content-secondary">
                    {message.payload.content}
                  </p>
                </div>
              </div>
            );
          })}
        </div>
      )}

      {onSend && (
        <div className="mt-3 border-t border-line-subtle pt-2">
          <div className="flex items-center gap-1.5">
            <select
              value={recipient}
              onChange={e => setRecipient(e.target.value)}
              aria-label={t('intelligence.teams.composer.recipient')}
              className="max-w-[40%] flex-none rounded-md border border-line bg-surface px-1.5 py-1 text-[11px] text-content-secondary">
              <option value="">{t('intelligence.teams.composer.toTeam')}</option>
              {members.map(m => (
                <option key={m.id} value={m.id}>
                  {m.name}
                </option>
              ))}
            </select>
            <input
              type="text"
              value={draft}
              disabled={sending}
              onChange={e => setDraft(e.target.value)}
              onKeyDown={e => {
                if (e.key === 'Enter') {
                  e.preventDefault();
                  submit();
                }
              }}
              placeholder={t('intelligence.teams.composer.placeholder')}
              className="min-w-0 flex-1 rounded-md border border-line px-2 py-1 text-[11px] text-content-secondary placeholder:text-stone-400 disabled:opacity-50 dark:bg-surface"
            />
            <button
              type="button"
              disabled={sending || draft.trim() === ''}
              onClick={submit}
              aria-label={t('intelligence.teams.composer.send')}
              title={t('intelligence.teams.composer.send')}
              className="inline-flex h-6 w-6 flex-none items-center justify-center rounded-md bg-ocean-500 text-white hover:bg-ocean-600 disabled:opacity-40">
              <LuSend className="h-3 w-3" />
            </button>
          </div>
        </div>
      )}
    </aside>
  );
}
