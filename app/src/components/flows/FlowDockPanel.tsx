/**
 * FlowDockPanel (Workflows UI redesign, Piece 1) — the tabbed right dock on
 * `FlowCanvasPage`, replacing the old mutually-exclusive Copilot-panel-vs-
 * Run-inspector-overlay split. Two tabs — Copilot | Run — share one docked,
 * resizable, collapsible panel.
 *
 * CRITICAL invariant: `copilotContent` (the `WorkflowCopilotPanel`) must stay
 * MOUNTED across tab switches — this component only toggles `display:none` on
 * the inactive tab's body, it never conditionally renders `copilotContent`
 * itself out of the tree. The host (`FlowCanvasPage`'s `FlowEditor`) must
 * likewise always pass the SAME element (not `activeTab === 'copilot' &&
 * <WorkflowCopilotPanel .../>`) — that's what preserves the agentic-task
 * panel's sticky-expand state (#4942) and the sub-agent `turnActive` flicker
 * fix (#5008/#5010) across a switch to the Run tab and back.
 */
import {
  type ReactNode,
  type PointerEvent as ReactPointerEvent,
  useCallback,
  useRef,
  useState,
} from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import Button from '../ui/Button';

export type FlowDockTab = 'copilot' | 'run';

export interface FlowDockPanelProps {
  activeTab: FlowDockTab;
  onTabChange: (tab: FlowDockTab) => void;
  /**
   * Rendered inside the Copilot tab's body. Always mounted regardless of
   * `activeTab` — see this file's doc comment.
   */
  copilotContent: ReactNode;
  /** Rendered inside the Run tab's body. Also always mounted (cheap to remount, but kept consistent with the copilot side). */
  runContent: ReactNode;
  /** Disables (and visually dims) the Run tab — a draft flow has no runs yet. */
  runTabDisabled?: boolean;
  /** Collapse the whole dock (the ▸ toggle) — hands control back to the host, which hides this component entirely. */
  onCollapse: () => void;
  /**
   * Render at full width with no resize handle (chat-first "graph appears
   * later" mode — mirrors the old `WorkflowCopilotPanel`'s `fullWidth` prop).
   */
  fullWidth?: boolean;
}

const MIN_WIDTH = 320;
const MAX_WIDTH = 560;
const DEFAULT_WIDTH = 384;

function ChevronRightIcon() {
  return (
    <svg
      className="h-4 w-4"
      fill="none"
      stroke="currentColor"
      viewBox="0 0 24 24"
      aria-hidden="true">
      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
    </svg>
  );
}

export default function FlowDockPanel({
  activeTab,
  onTabChange,
  copilotContent,
  runContent,
  runTabDisabled = false,
  onCollapse,
  fullWidth = false,
}: FlowDockPanelProps) {
  const { t } = useT();
  const [width, setWidth] = useState(DEFAULT_WIDTH);
  const draggingRef = useRef(false);

  const handleResizeStart = useCallback(
    (event: ReactPointerEvent) => {
      event.preventDefault();
      draggingRef.current = true;
      const startX = event.clientX;
      const startWidth = width;

      const onMove = (moveEvent: PointerEvent) => {
        if (!draggingRef.current) return;
        // The dock sits on the canvas's RIGHT edge — dragging its left-edge
        // resize handle LEFT (negative clientX delta) grows the dock.
        const delta = startX - moveEvent.clientX;
        setWidth(Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, startWidth + delta)));
      };
      const onUp = () => {
        draggingRef.current = false;
        window.removeEventListener('pointermove', onMove);
        window.removeEventListener('pointerup', onUp);
      };
      window.addEventListener('pointermove', onMove);
      window.addEventListener('pointerup', onUp);
    },
    [width]
  );

  const tabs: Array<{ key: FlowDockTab; label: string; testId: string }> = [
    { key: 'copilot', label: t('flows.dock.copilotTab'), testId: 'flow-dock-tab-copilot' },
    { key: 'run', label: t('flows.dock.runTab'), testId: 'flow-dock-tab-run' },
  ];

  return (
    <div
      className="relative flex h-full flex-shrink-0 border-l border-line bg-surface"
      style={fullWidth ? undefined : { width }}
      data-testid="flow-dock-panel"
      data-full-width={fullWidth ? 'true' : 'false'}>
      {!fullWidth && (
        <div
          role="separator"
          aria-orientation="vertical"
          aria-label={t('flows.dock.resizeHandle')}
          data-testid="flow-dock-resize-handle"
          onPointerDown={handleResizeStart}
          className="absolute -left-1 top-0 z-10 h-full w-2 cursor-col-resize"
        />
      )}
      <div className={`flex h-full min-w-0 flex-1 flex-col ${fullWidth ? 'w-full' : ''}`}>
        <div className="flex flex-shrink-0 items-center justify-between gap-2 border-b border-line px-2 py-1.5">
          <div
            role="tablist"
            aria-label={t('flows.dock.tablistLabel')}
            className="inline-flex items-center gap-0.5 rounded-lg bg-surface-muted p-0.5">
            {tabs.map(tab => {
              const disabled = tab.key === 'run' && runTabDisabled;
              const active = activeTab === tab.key;
              return (
                <button
                  key={tab.key}
                  type="button"
                  role="tab"
                  aria-selected={active}
                  disabled={disabled}
                  data-testid={tab.testId}
                  onClick={() => onTabChange(tab.key)}
                  className={`rounded-md px-2.5 py-1 text-xs font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-40 ${
                    active
                      ? 'bg-surface text-content shadow-sm'
                      : 'text-content-secondary hover:text-content'
                  }`}>
                  {tab.label}
                </button>
              );
            })}
          </div>
          <Button
            type="button"
            variant="tertiary"
            size="xs"
            iconOnly
            data-testid="flow-dock-collapse"
            aria-label={t('flows.dock.collapse')}
            title={t('flows.dock.collapse')}
            onClick={onCollapse}>
            <ChevronRightIcon />
          </Button>
        </div>

        {/* Both bodies are always in the tree — only `display` toggles — so the
            Copilot's internal state survives switching to Run and back. */}
        <div
          className="min-h-0 flex-1"
          data-testid="flow-dock-copilot-body"
          style={{ display: activeTab === 'copilot' ? 'flex' : 'none' }}>
          {copilotContent}
        </div>
        <div
          className="min-h-0 flex-1 overflow-y-auto"
          data-testid="flow-dock-run-body"
          style={{ display: activeTab === 'run' ? 'block' : 'none' }}>
          {runContent}
        </div>
      </div>
    </div>
  );
}
