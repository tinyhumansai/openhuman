/**
 * ActionItemChecklist — renders a list of MeetCallActionItem objects.
 *
 * Executable items show a "Run with OpenHuman" button that navigates to /chat.
 * Advisory items show only the description + metadata.
 * Checked state is cosmetic (local only, not persisted).
 */
import debug from 'debug';
import { useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { useT } from '../../lib/i18n/I18nContext';
import type { MeetCallActionItem } from '../../services/meetCallService';
import Button from '../ui/Button';

const log = debug('meetings:action');

interface ActionItemChecklistProps {
  items: MeetCallActionItem[];
}

export function ActionItemChecklist({ items }: ActionItemChecklistProps) {
  const { t } = useT();
  const navigate = useNavigate();
  const [checked, setChecked] = useState<Record<number, boolean>>({});

  if (items.length === 0) return null;

  function handleCheck(index: number) {
    setChecked(prev => ({ ...prev, [index]: !prev[index] }));
  }

  function handleRun(item: MeetCallActionItem) {
    log('[action] run with OpenHuman clicked', {
      description: item.description,
      tool: item.tool_name,
    });
    // TODO: prefill chat with action item description — prefill not yet supported
    void navigate('/chat');
  }

  return (
    <div className="space-y-1">
      <p className="text-[10px] font-semibold uppercase tracking-wide text-content-muted">
        {t('skills.meetingBots.callActionItemsHeading')}
      </p>
      <ul className="mt-0.5 space-y-1.5 text-[11px]">
        {items.map((item, i) => {
          const isExecutable = item.kind === 'executable';
          const meta = [
            item.assignee?.trim() || undefined,
            isExecutable ? item.tool_name?.trim() || undefined : undefined,
          ].filter(Boolean);

          return (
            <li key={i} className="flex items-start gap-2">
              <input
                type="checkbox"
                checked={!!checked[i]}
                onChange={() => handleCheck(i)}
                aria-label={item.description}
                className="mt-0.5 h-3 w-3 shrink-0 cursor-pointer rounded accent-primary-600"
              />
              <div className="min-w-0 flex-1">
                <span
                  className={
                    checked[i] ? 'text-content-faint line-through' : 'text-content-secondary'
                  }>
                  {item.description}
                </span>
                {meta.length > 0 && (
                  <span className="ml-1 text-content-faint text-[10px]">({meta.join(' · ')})</span>
                )}
                {isExecutable && (
                  <span className="ml-2">
                    <Button variant="tertiary" size="xs" onClick={() => handleRun(item)}>
                      {t('skills.meetingBots.history.runWithOpenHuman')}
                    </Button>
                  </span>
                )}
              </div>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

export default ActionItemChecklist;
