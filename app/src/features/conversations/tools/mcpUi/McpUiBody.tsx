import { isTauri } from '../../../../utils/tauriCommands/common';
import { LinkActions } from './LinkActions';
import { hasWidget, readMcpUiPresentation } from './types';

/**
 * What a tool call that offered UI shows under its card: the links the result
 * offered, unless its widget is shown on its own after the activity group.
 */
export function McpUiBody({ structured }: { structured: unknown }) {
  const presentation = readMcpUiPresentation(structured);
  if (!presentation) return null;
  if (hasWidget(presentation) && isTauri()) return null;
  if (presentation.links.length === 0) return null;
  return (
    <div className="flex flex-col gap-2" data-testid="mcp-ui-body">
      <LinkActions links={presentation.links} />
    </div>
  );
}
