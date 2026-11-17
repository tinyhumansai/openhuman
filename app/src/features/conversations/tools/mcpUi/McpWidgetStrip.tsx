import { isTauri } from '../../../../utils/tauriCommands/common';
import { McpAppFrame } from './McpAppFrame';
import type { MessageWidget } from './messageWidgets';

/** Widgets shown on their own, between the work that produced them and the answer. */
export function McpWidgetStrip({ widgets }: { widgets: readonly MessageWidget[] }) {
  if (widgets.length === 0 || !isTauri()) return null;
  return (
    <div className="flex flex-col gap-3" data-slot="aui_mcp-widgets" data-testid="mcp-ui-widgets">
      {widgets.map(widget => (
        <McpAppFrame key={widget.key} presentation={widget.presentation} />
      ))}
    </div>
  );
}
