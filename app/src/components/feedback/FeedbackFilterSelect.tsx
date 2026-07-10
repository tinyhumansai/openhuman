import { useEffect, useId, useRef, useState } from 'react';

export interface FilterOption {
  value: string;
  label: string;
}

interface FeedbackFilterSelectProps {
  value: string;
  options: FilterOption[];
  onChange: (value: string) => void;
  /** Accessible label for the trigger button. */
  ariaLabel: string;
}

/**
 * Lightweight styled dropdown for the board filters. Native `<select>` is hard
 * to theme consistently across platforms, so this renders a button + popover
 * with the app's tokens. It honours the listbox ARIA contract: the open popover
 * takes focus and tracks a highlighted option via `aria-activedescendant`, with
 * Up/Down/Home/End to move, Enter/Space to select, Escape + outside-click to
 * dismiss. Keeping focus on the listbox (rather than roving it across option
 * buttons) means every keystroke lands on one handler regardless of how the
 * popover was opened.
 */
export default function FeedbackFilterSelect({
  value,
  options,
  onChange,
  ariaLabel,
}: FeedbackFilterSelectProps) {
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(-1);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const baseId = useId();
  const optionId = (index: number) => `${baseId}-option-${index}`;

  const selectedIndex = Math.max(
    0,
    options.findIndex(option => option.value === value)
  );

  const openMenu = () => {
    setActiveIndex(selectedIndex);
    setOpen(true);
  };

  const closeMenu = (focusTrigger = false) => {
    setOpen(false);
    if (focusTrigger) triggerRef.current?.focus();
  };

  const selectOption = (option: FilterOption) => {
    onChange(option.value);
    closeMenu(true);
  };

  // Dismiss on outside-click or Escape while the popover is open.
  useEffect(() => {
    if (!open) return undefined;
    const onPointerDown = (event: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(event.target as Node)) setOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') closeMenu(true);
    };
    document.addEventListener('mousedown', onPointerDown);
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('mousedown', onPointerDown);
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [open]);

  // Move focus into the popover when it opens so keystrokes target the listbox.
  useEffect(() => {
    if (open) listRef.current?.focus();
  }, [open]);

  const onTriggerKeyDown = (event: React.KeyboardEvent<HTMLButtonElement>) => {
    if (!open && (event.key === 'ArrowDown' || event.key === 'ArrowUp')) {
      event.preventDefault();
      openMenu();
    }
  };

  const onListKeyDown = (event: React.KeyboardEvent<HTMLUListElement>) => {
    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault();
        setActiveIndex(index => (index + 1) % options.length);
        break;
      case 'ArrowUp':
        event.preventDefault();
        setActiveIndex(index => (index - 1 + options.length) % options.length);
        break;
      case 'Home':
        event.preventDefault();
        setActiveIndex(0);
        break;
      case 'End':
        event.preventDefault();
        setActiveIndex(options.length - 1);
        break;
      case 'Enter':
      case ' ':
        event.preventDefault();
        if (activeIndex >= 0) selectOption(options[activeIndex]);
        break;
      case 'Escape':
        event.preventDefault();
        closeMenu(true);
        break;
      default:
        break;
    }
  };

  const current = options.find(option => option.value === value) ?? options[0];

  return (
    <div ref={rootRef} className="relative">
      <button
        ref={triggerRef}
        type="button"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={ariaLabel}
        onClick={() => (open ? closeMenu() : openMenu())}
        onKeyDown={onTriggerKeyDown}
        className={`flex items-center gap-1.5 rounded-lg border px-3 py-1.5 text-xs font-medium transition-colors ${
          open
            ? 'border-primary-500/50 bg-surface text-content ring-2 ring-primary-500/20'
            : 'border-line bg-surface-muted text-content-secondary hover:border-line-strong hover:text-content dark:border-line-strong dark:bg-white/[0.03] dark:hover:text-content'
        }`}>
        {current?.label}
        <svg
          className={`h-3.5 w-3.5 transition-transform ${open ? 'rotate-180' : ''}`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
        </svg>
      </button>

      {open && (
        <ul
          ref={listRef}
          role="listbox"
          tabIndex={-1}
          aria-label={ariaLabel}
          aria-activedescendant={activeIndex >= 0 ? optionId(activeIndex) : undefined}
          onKeyDown={onListKeyDown}
          className="absolute z-20 mt-1.5 min-w-[10rem] animate-scale-in overflow-hidden rounded-xl border border-line bg-surface p-1 shadow-medium focus:outline-none dark:border-line-strong">
          {options.map((option, index) => {
            const selected = option.value === value;
            const active = index === activeIndex;
            return (
              <li key={option.value} id={optionId(index)} role="option" aria-selected={selected}>
                <button
                  type="button"
                  tabIndex={-1}
                  onClick={() => selectOption(option)}
                  onMouseEnter={() => setActiveIndex(index)}
                  className={`flex w-full items-center justify-between gap-3 rounded-lg px-3 py-1.5 text-left text-xs transition-colors ${
                    selected
                      ? 'font-medium text-primary-600 dark:text-primary-400'
                      : 'text-content-secondary'
                  } ${
                    active
                      ? 'bg-surface-subtle dark:bg-white/[0.08]'
                      : selected
                        ? 'bg-primary-500/10'
                        : ''
                  }`}>
                  {option.label}
                  {selected && (
                    <svg
                      className="h-3.5 w-3.5"
                      fill="none"
                      stroke="currentColor"
                      viewBox="0 0 24 24">
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        strokeWidth={2.2}
                        d="M5 13l4 4L19 7"
                      />
                    </svg>
                  )}
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
