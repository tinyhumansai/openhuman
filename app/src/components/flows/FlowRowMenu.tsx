/**
 * FlowRowMenu — the "⋯" overflow menu on a Workflows list row. Holds the
 * secondary/rare actions (Export, Duplicate, Delete) so the row's primary
 * actions (View runs, Run) stay uncluttered, and keeps the destructive Delete
 * out of the flat button row. Closes on Escape, outside click, or item select.
 *
 * The menu is rendered in a portal with fixed positioning so it escapes the
 * list card's `overflow-hidden` clipping — otherwise the last row's menu would
 * be cut off by the card edge. It flips above the button when there isn't room
 * below in the viewport.
 *
 * Presentational + local open state only — each item calls back up to
 * `FlowListRow`, which routes to `FlowsPage`'s handlers.
 */
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

import { useEscapeKey } from '../../hooks/useEscapeKey';
import { useT } from '../../lib/i18n/I18nContext';

export interface FlowRowMenuItem {
  key: string;
  label: string;
  onSelect: () => void;
  /** Renders the item in the destructive coral tone (e.g. Delete). */
  danger?: boolean;
  testId?: string;
}

export interface FlowRowMenuProps {
  items: FlowRowMenuItem[];
  /** Suffixed onto test ids so multiple rows stay addressable. */
  rowId: string;
}

function KebabIcon() {
  return (
    <svg className="h-4 w-4" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <circle cx="12" cy="5" r="1.6" />
      <circle cx="12" cy="12" r="1.6" />
      <circle cx="12" cy="19" r="1.6" />
    </svg>
  );
}

const MENU_WIDTH = 160; // matches min-w-[10rem]

export default function FlowRowMenu({ items, rowId }: FlowRowMenuProps) {
  const { t } = useT();
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);
  const buttonRef = useRef<HTMLButtonElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);

  useEscapeKey(() => setOpen(false), open);

  // Position the portaled menu in viewport coords so it escapes the list card's
  // `overflow-hidden`. Flip above the button when there isn't room below.
  const place = useCallback(() => {
    const btn = buttonRef.current;
    if (!btn) return;
    const rect = btn.getBoundingClientRect();
    const menuH = menuRef.current?.offsetHeight ?? 8 + items.length * 30;
    const openAbove = rect.bottom + menuH + 8 > window.innerHeight && rect.top - menuH - 4 > 0;
    setPos({
      top: openAbove ? rect.top - menuH - 4 : rect.bottom + 4,
      left: Math.max(8, rect.right - MENU_WIDTH),
    });
  }, [items.length]);

  useLayoutEffect(() => {
    if (open) place();
  }, [open, place]);

  // Reposition on scroll/resize while open.
  useEffect(() => {
    if (!open) return;
    const reposition = () => place();
    window.addEventListener('resize', reposition);
    window.addEventListener('scroll', reposition, true);
    return () => {
      window.removeEventListener('resize', reposition);
      window.removeEventListener('scroll', reposition, true);
    };
  }, [open, place]);

  // Close on any click outside the button and the (portaled) menu.
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: MouseEvent) => {
      const target = event.target as Node | null;
      if (buttonRef.current?.contains(target) || menuRef.current?.contains(target)) return;
      setOpen(false);
    };
    document.addEventListener('mousedown', onPointerDown);
    return () => document.removeEventListener('mousedown', onPointerDown);
  }, [open]);

  const select = useCallback((onSelect: () => void) => {
    setOpen(false);
    onSelect();
  }, []);

  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        data-testid={`flow-menu-${rowId}`}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={t('flows.list.moreActions')}
        title={t('flows.list.moreActions')}
        onClick={() => setOpen(o => !o)}
        className="flex h-8 w-8 items-center justify-center rounded-lg border border-line text-content-muted transition-colors hover:bg-surface-hover hover:text-content-secondary">
        <KebabIcon />
      </button>

      {open &&
        createPortal(
          <div
            ref={menuRef}
            role="menu"
            data-testid={`flow-menu-list-${rowId}`}
            style={{
              position: 'fixed',
              top: pos?.top ?? -9999,
              left: pos?.left ?? -9999,
              minWidth: '10rem',
            }}
            className="z-50 overflow-hidden rounded-xl border border-line bg-surface py-1 shadow-lg">
            {items.map(item => (
              <button
                key={item.key}
                type="button"
                role="menuitem"
                data-testid={item.testId}
                onClick={() => select(item.onSelect)}
                className={`block w-full px-3 py-1.5 text-left text-xs transition-colors hover:bg-surface-hover ${
                  item.danger ? 'text-coral-600 dark:text-coral-400' : 'text-content-secondary'
                }`}>
                {item.label}
              </button>
            ))}
          </div>,
          document.body
        )}
    </>
  );
}
