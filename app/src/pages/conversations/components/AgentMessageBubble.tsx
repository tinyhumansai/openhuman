import Markdown from 'react-markdown';

import { OPENHUMAN_LINK_EVENT } from '../../../components/OpenhumanLinkModal';
import { parseMarkdownTable } from '../../../utils/agentMessageBubbles';
import { openUrl } from '../../../utils/openUrl';
import {
  type AgentBubblePosition,
  getAgentBubbleChrome,
  isAllowedExternalHref,
  parseBubbleSegments,
} from '../utils/format';

/**
 * Pill rendered below an agent bubble for each
 * `<openhuman-link path="...">label</openhuman-link>` tag the agent
 * emits. Click dispatches an `OPENHUMAN_LINK_EVENT` window event that
 * `OpenhumanLinkModal` listens for, so the chat stays in view.
 */
function OpenhumanLinkPill({ path, label }: { path: string; label: string }) {
  return (
    <button
      type="button"
      onClick={() =>
        window.dispatchEvent(new CustomEvent(OPENHUMAN_LINK_EVENT, { detail: { path } }))
      }
      className="inline-flex items-center gap-1 rounded-full border border-primary-200 bg-primary-50 px-3 py-1 text-xs font-medium text-primary-700 transition-colors hover:bg-primary-100">
      {label}
      <svg className="h-3 w-3" viewBox="0 0 24 24" fill="none" stroke="currentColor">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M5 12h14M13 6l6 6-6 6"
        />
      </svg>
    </button>
  );
}

export function BubbleMarkdown({
  content,
  tone = 'agent',
}: {
  content: string;
  tone?: 'agent' | 'user';
}) {
  const proseTone =
    tone === 'user'
      ? 'prose-invert prose-p:text-white prose-li:text-white prose-a:text-white prose-code:text-white prose-strong:text-white prose-headings:text-white [&_li::marker]:text-white/85'
      : 'prose-a:text-primary-500 prose-code:text-primary-700 prose-headings:text-sm [&_li::marker]:text-stone-700';

  return (
    <div
      className={`text-sm prose prose-sm max-w-none prose-p:my-1 prose-pre:my-2 prose-pre:rounded-lg prose-code:text-xs prose-headings:font-semibold prose-ul:my-0 prose-ol:my-0 prose-li:my-0 ${proseTone} ${
        tone === 'user' ? 'prose-pre:bg-white/10' : 'prose-pre:bg-stone-300/50'
      } [&_ul]:my-0 [&_ol]:my-0 [&_ul]:pl-0 [&_ol]:pl-0 [&_ul]:list-inside [&_ol]:list-inside [&_li]:my-0 [&_li]:pl-0 [&_li_p]:inline [&_li_p]:m-0`}>
      <Markdown
        components={{
          a: ({ href, children }) => (
            <a
              href={href}
              onClick={e => {
                e.preventDefault();
                if (!href || !isAllowedExternalHref(href)) return;
                void openUrl(href).catch(() => {
                  // Ignore launcher errors from OS URL handler failures.
                });
              }}
              className="cursor-pointer underline">
              {children}
            </a>
          ),
        }}>
        {content}
      </Markdown>
    </div>
  );
}

export function TableCellMarkdown({ content }: { content: string }) {
  return (
    <div className="prose prose-sm max-w-none text-sm text-stone-700 prose-p:my-0 prose-ul:my-0 prose-ol:my-0 prose-li:my-0 prose-code:text-xs prose-code:text-primary-700 prose-a:text-primary-500 prose-strong:text-stone-900 prose-headings:text-sm prose-headings:font-semibold [&_li::marker]:text-stone-700 [&_ul]:my-0 [&_ol]:my-0 [&_ul]:pl-0 [&_ol]:pl-0 [&_ul]:list-inside [&_ol]:list-inside [&_li]:pl-0 [&_li_p]:inline [&_li_p]:m-0">
      <Markdown
        components={{
          a: ({ href, children }) => (
            <a
              href={href}
              onClick={e => {
                e.preventDefault();
                if (!href || !isAllowedExternalHref(href)) return;
                void openUrl(href).catch(() => {
                  // Ignore launcher errors from OS URL handler failures.
                });
              }}
              className="cursor-pointer underline">
              {children}
            </a>
          ),
        }}>
        {content}
      </Markdown>
    </div>
  );
}

export function AgentMessageBubble({
  content,
  position = 'single',
}: {
  content: string;
  position?: AgentBubblePosition;
}) {
  const segments = parseBubbleSegments(content);
  const textContent = segments
    .filter(s => s.kind === 'text')
    .map(s => s.text)
    .join('')
    .trim();
  const linkSegments = segments.filter(
    (s): s is Extract<typeof s, { kind: 'link' }> => s.kind === 'link'
  );

  const table = parseMarkdownTable(textContent);
  const bubbleChrome = getAgentBubbleChrome(position);

  if (table) {
    return (
      <div
        className={`w-full max-w-full overflow-hidden border border-stone-200 bg-white/90 shadow-sm ${bubbleChrome}`}>
        <div className="overflow-x-auto">
          <table className="w-max min-w-full border-collapse text-left text-sm text-stone-800">
            <thead className="bg-stone-100/90">
              <tr>
                {table.headers.map(header => (
                  <th
                    key={header}
                    className="max-w-[25vw] border-b border-stone-200 px-4 py-2.5 text-xs font-semibold uppercase tracking-[0.08em] text-stone-500">
                    {header}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {table.rows.map((row, rowIndex) => (
                <tr
                  key={`${rowIndex}:${row.join('|')}`}
                  className="odd:bg-white even:bg-stone-50/70">
                  {row.map((cell, cellIndex) => (
                    <td
                      key={`${rowIndex}:${cellIndex}:${cell}`}
                      className="max-w-[25vw] border-t border-stone-200 px-4 py-3 align-top text-sm text-stone-700">
                      <TableCellMarkdown content={cell} />
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
    );
  }

  return (
    <>
      {textContent && (
        <div className={`bg-stone-200/80 px-4 py-2.5 text-stone-900 ${bubbleChrome}`}>
          <BubbleMarkdown content={textContent} />
        </div>
      )}
      {linkSegments.length > 0 && (
        <div className="mt-2 flex flex-wrap gap-2">
          {linkSegments.map((segment, idx) => (
            <OpenhumanLinkPill
              key={`pill-${idx}-${segment.path}`}
              path={segment.path}
              label={segment.label}
            />
          ))}
        </div>
      )}
    </>
  );
}
