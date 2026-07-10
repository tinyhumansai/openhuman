/**
 * Collapsible list of MCP tools with name and description.
 *
 * Optionally renders a per-tool "Try" button when `onTryTool` is
 * provided — clicking it hands the selected tool back to the parent so
 * it can open the Tool Execution Playground. When the prop is absent
 * the list stays purely informational (preserving the original API for
 * any other call site).
 */
import { useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import type { McpTool } from './types';

interface McpToolListProps {
  tools: McpTool[];
  /** When provided, each tool gets a "Try" button that calls this with that tool. */
  onTryTool?: (tool: McpTool) => void;
}

const McpToolList = ({ tools, onTryTool }: McpToolListProps) => {
  const { t } = useT();
  const [expanded, setExpanded] = useState(false);
  // Guard against undefined/null passed at runtime (TypeScript can't always prevent this).
  const safeTools = tools ?? [];

  if (safeTools.length === 0) {
    return <p className="text-xs text-content-faint">{t('mcp.toolList.noTools')}</p>;
  }

  return (
    <div className="space-y-1">
      <button
        type="button"
        onClick={() => setExpanded(prev => !prev)}
        className="flex items-center gap-1.5 text-xs font-medium text-content-secondary hover:text-content">
        <span className={`transition-transform ${expanded ? 'rotate-90' : ''}`} aria-hidden="true">
          ▶
        </span>
        {t(
          safeTools.length === 1 ? 'mcp.toolList.availableSingular' : 'mcp.toolList.availablePlural'
        ).replace('{count}', String(safeTools.length))}
      </button>

      {expanded && (
        <ul className="mt-2 space-y-1 pl-4 border-l-2 border-line-subtle">
          {safeTools.map(tool => (
            <li key={tool.name} className="space-y-0.5">
              <div className="flex items-start justify-between gap-2">
                <p className="text-xs font-mono font-medium text-content break-words min-w-0">
                  {tool.name}
                </p>
                {onTryTool && (
                  <button
                    type="button"
                    onClick={() => onTryTool(tool)}
                    aria-label={t('mcp.toolList.tryToolAria').replace('{name}', tool.name)}
                    className="shrink-0 text-[10px] font-medium text-primary-600 dark:text-primary-300 hover:underline">
                    {t('mcp.toolList.tryTool')}
                  </button>
                )}
              </div>
              {tool.description && (
                <p className="text-[11px] text-content-muted">{tool.description}</p>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
};

export default McpToolList;
