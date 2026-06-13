/**
 * Compact memory citation chips for assistant messages (wired from
 * `extraMetadata.citations` populated on `chat_done` / segment events).
 */
export type MessageCitation = {
  id: string;
  key: string;
  namespace?: string;
  score?: number;
  timestamp: string;
  snippet: string;
};

export function CitationChips({ citations }: { citations: MessageCitation[] }) {
  const filteredCitations = citations.filter(c => c.namespace !== 'global');
  if (filteredCitations.length === 0) return null;
  return (
    <div className="mt-1.5 flex flex-wrap gap-1">
      {filteredCitations.map(citation => {
        const scoreLabel =
          typeof citation.score === 'number' ? ` ${Math.round(citation.score * 100)}%` : '';
        const title = `${citation.key}${citation.namespace ? ` (${citation.namespace})` : ''}\n${citation.snippet}`;
        return (
          <details key={citation.id} className="group">
            <summary
              className="list-none cursor-pointer rounded-full border border-stone-300 dark:border-neutral-700 bg-stone-100 dark:bg-neutral-800 px-2 py-0.5 text-[10px] text-stone-600 dark:text-neutral-300 hover:bg-stone-200 dark:hover:bg-neutral-700"
              aria-label={title}
              title={title}>
              {citation.namespace ?? citation.key}
              {scoreLabel}
            </summary>
            <div className="mt-1 max-w-md rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-2 py-1 text-[11px] text-stone-600 dark:text-neutral-300 shadow-sm">
              {citation.snippet}
            </div>
          </details>
        );
      })}
    </div>
  );
}
