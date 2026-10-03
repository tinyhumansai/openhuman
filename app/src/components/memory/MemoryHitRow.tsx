/**
 * One stored item — a recall citation, a raw fetch hit, or a learning — as a
 * list row: kind badge, optional score, the text, and the metadata that says
 * where it came from (file, folder, repo, thread, link).
 */
import type { ReactNode } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { ItemKind, MemoryMeta } from '../../services/api/memoryApi';
import { Badge } from '../ui';
import { formatScore, KIND_VARIANT, kindLabel, metaFacts } from './memoryFormat';

interface MemoryHitRowProps {
  id: string;
  kind: ItemKind;
  /** The hit's text or the citation's snippet. */
  text: string;
  meta: MemoryMeta;
  score?: number | null;
  /** Show the score badge (raw results); recall citations and learnings hide it. */
  showScore?: boolean;
  /** Right-hand action, e.g. a delete button. */
  action?: ReactNode;
  'data-testid'?: string;
}

export default function MemoryHitRow({
  id,
  kind,
  text,
  meta,
  score,
  showScore = false,
  action,
  'data-testid': testId,
}: MemoryHitRowProps) {
  const { t } = useT();
  const facts = metaFacts(meta, t);
  const scoreText = showScore ? formatScore(score) : null;

  return (
    <li
      className="flex items-start gap-3 px-4 py-3"
      data-testid={testId ?? `memory-hit-${id}`}
      data-kind={kind}>
      <div className="min-w-0 flex-1 space-y-1.5">
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant={KIND_VARIANT[kind] ?? 'neutral'}>{kindLabel(kind, t)}</Badge>
          {scoreText !== null && (
            <span
              className="font-mono text-[11px] text-content-muted"
              data-testid="memory-hit-score">
              {t('memoryPage.score')} {scoreText}
            </span>
          )}
        </div>
        <p className="whitespace-pre-wrap break-words text-sm text-content">{text}</p>
        {facts.length > 0 && (
          <dl className="flex flex-wrap gap-x-4 gap-y-0.5 text-[11px] text-content-muted">
            {facts.map(fact => (
              <div
                key={fact.key}
                className="flex min-w-0 gap-1"
                data-testid={`memory-meta-${fact.key}`}>
                <dt className="shrink-0 font-medium">{fact.label}</dt>
                <dd className="truncate font-mono">{fact.value}</dd>
              </div>
            ))}
          </dl>
        )}
      </div>
      {action ? <div className="shrink-0">{action}</div> : null}
    </li>
  );
}
