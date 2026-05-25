/**
 * Collapsible list of MCP tools with name and description.
 */
import { useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import type { McpTool } from './types';

interface McpToolListProps {
  tools: McpTool[];
}

const McpToolList = ({ tools }: McpToolListProps) => {
  const { t } = useT();
  const [expanded, setExpanded] = useState(false);
  // Guard against undefined/null passed at runtime (TypeScript can't always prevent this).
  const safeTools = tools ?? [];

  if (safeTools.length === 0) {
    return (
      <p className="text-xs text-stone-400 dark:text-neutral-500">{t('mcp.toolList.noTools')}</p>
    );
  }

  return (
    <div className="space-y-1">
      <button
        type="button"
        onClick={() => setExpanded(prev => !prev)}
        className="flex items-center gap-1.5 text-xs font-medium text-stone-600 dark:text-neutral-300 hover:text-stone-900 dark:hover:text-neutral-100">
        <span className={`transition-transform ${expanded ? 'rotate-90' : ''}`} aria-hidden="true">
          ▶
        </span>
        {t(
          safeTools.length === 1 ? 'mcp.toolList.availableSingular' : 'mcp.toolList.availablePlural'
        ).replace('{count}', String(safeTools.length))}
      </button>

      {expanded && (
        <ul className="mt-2 space-y-1 pl-4 border-l-2 border-stone-100 dark:border-neutral-800">
          {safeTools.map(tool => (
            <li key={tool.name} className="space-y-0.5">
              <p className="text-xs font-mono font-medium text-stone-800 dark:text-neutral-100">
                {tool.name}
              </p>
              {tool.description && (
                <p className="text-[11px] text-stone-500 dark:text-neutral-400">
                  {tool.description}
                </p>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
};

export default McpToolList;
