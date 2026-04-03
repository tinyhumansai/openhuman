import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

import type { ActionableItem, SnoozeOption } from '../../types/intelligence';

interface ActionableCardProps {
  item: ActionableItem;
  onComplete: (item: ActionableItem) => void;
  onDismiss: (item: ActionableItem) => void;
  onSnooze: (item: ActionableItem, duration: number) => void;
  className?: string;
}

const SNOOZE_OPTIONS: SnoozeOption[] = [
  { label: '1 hour', duration: 60 * 60 * 1000 },
  { label: '6 hours', duration: 6 * 60 * 60 * 1000 },
  { label: '24 hours', duration: 24 * 60 * 60 * 1000 },
];

// Portal component for snooze dropdown to escape stacking contexts
interface SnoozeDropdownPortalProps {
  isOpen: boolean;
  buttonRef: React.RefObject<HTMLButtonElement | null>;
  onClose: () => void;
  onSnooze: (duration: number) => void;
}

function SnoozeDropdownPortal({ isOpen, buttonRef, onClose, onSnooze }: SnoozeDropdownPortalProps) {
  const [position, setPosition] = useState({ top: 0, left: 0 });
  const dropdownRef = useRef<HTMLDivElement>(null);

  // Calculate position based on button position
  useEffect(() => {
    if (isOpen && buttonRef.current) {
      const rect = buttonRef.current.getBoundingClientRect();
      const dropdownWidth = 120;

      // Position dropdown below and aligned to right edge of button
      const left = Math.max(8, rect.right - dropdownWidth);
      const top = rect.bottom + 4;

      setPosition({ top, left });
    }
  }, [isOpen, buttonRef]);

  // Handle click outside to close dropdown
  useEffect(() => {
    if (!isOpen) return;

    const handleClickOutside = (event: MouseEvent) => {
      const target = event.target as Element;

      // Don't close if clicking the button or dropdown itself
      if (
        buttonRef.current?.contains(target) ||
        dropdownRef.current?.contains(target) ||
        target.closest('[data-snooze-dropdown]')
      ) {
        return;
      }

      onClose();
    };

    // Use capture phase to ensure we handle this before other click handlers
    document.addEventListener('click', handleClickOutside, true);
    return () => document.removeEventListener('click', handleClickOutside, true);
  }, [isOpen, onClose, buttonRef]);

  // Handle escape key
  useEffect(() => {
    if (!isOpen) return;

    const handleEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        onClose();
      }
    };

    document.addEventListener('keydown', handleEscape);
    return () => document.removeEventListener('keydown', handleEscape);
  }, [isOpen, onClose]);

  if (!isOpen) return null;

  return createPortal(
    <div
      ref={dropdownRef}
      data-snooze-dropdown
      className="fixed py-1 bg-white border border-stone-200 rounded-lg shadow-xl min-w-[120px] z-[9999] animate-fade-in"
      style={{ top: position.top, left: position.left }}>
      {SNOOZE_OPTIONS.map(option => (
        <button
          key={option.label}
          onClick={() => onSnooze(option.duration)}
          className="w-full text-left px-3 py-1.5 text-xs text-stone-900 hover:bg-stone-100 transition-colors cursor-pointer">
          {option.label}
        </button>
      ))}
    </div>,
    document.body
  );
}

// Source icons for different actionable item types
const SOURCE_ICONS = {
  email: (
    <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M3 8l7.89 4.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"
      />
    </svg>
  ),
  calendar: (
    <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M8 7V3m8 4V3m-9 8h10M5 21h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v14a2 2 0 002 2z"
      />
    </svg>
  ),
  telegram: (
    <svg className="w-4 h-4" fill="currentColor" viewBox="0 0 24 24">
      <path d="M11.944 0A12 12 0 0 0 0 12a12 12 0 0 0 12 12 12 12 0 0 0 12-12A12 12 0 0 0 12 0a12 12 0 0 0-.056 0zm4.962 7.224c.1-.002.321.023.465.14a.506.506 0 0 1 .171.325c.016.093.036.306.02.472-.18 1.898-.962 6.502-1.36 8.627-.168.9-.499 1.201-.82 1.23-.696.065-1.225-.46-1.9-.902-1.056-.693-1.653-1.124-2.678-1.8-1.185-.78-.417-1.21.258-1.91.177-.184 3.247-2.977 3.307-3.23.007-.032.014-.15-.056-.212s-.174-.041-.249-.024c-.106.024-1.793 1.14-5.061 3.345-.48.33-.913.49-1.302.48-.428-.008-1.252-.241-1.865-.44-.752-.245-1.349-.374-1.297-.789.027-.216.325-.437.893-.663 3.498-1.524 5.83-2.529 6.998-3.014 3.332-1.386 4.025-1.627 4.476-1.635z" />
    </svg>
  ),
  ai_insight: (
    <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z"
      />
    </svg>
  ),
  system: (
    <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"
      />
    </svg>
  ),
  trading: (
    <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M13 7h8m0 0v8m0-8l-8 8-4-4-6 6"
      />
    </svg>
  ),
  security: (
    <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z"
      />
    </svg>
  ),
};

function formatTimeAgo(date: Date): string {
  const now = new Date();
  const diff = now.getTime() - date.getTime();

  const minutes = Math.floor(diff / (1000 * 60));
  const hours = Math.floor(diff / (1000 * 60 * 60));
  const days = Math.floor(diff / (1000 * 60 * 60 * 24));
  const weeks = Math.floor(diff / (1000 * 60 * 60 * 24 * 7));

  if (minutes < 1) return 'Just now';
  if (minutes < 60) return `${minutes} min${minutes === 1 ? '' : 's'} ago`;
  if (hours < 24) {
    if (hours === 1) return '1 hour ago';
    if (hours <= 6) return `${hours} hours ago`;
    if (hours <= 12) return 'This morning';
    return 'This afternoon';
  }
  if (days === 1) return 'Yesterday';
  if (days < 7) return `${days} days ago`;
  if (weeks === 1) return '1 week ago';
  return `${weeks} weeks ago`;
}

function isNewItem(date: Date): boolean {
  const now = new Date();
  const diff = now.getTime() - date.getTime();
  return diff < 5 * 60 * 1000; // Less than 5 minutes old
}

export function ActionableCard({
  item,
  onComplete,
  onDismiss,
  onSnooze,
  className = '',
}: ActionableCardProps) {
  const [showSnoozeMenu, setShowSnoozeMenu] = useState(false);
  const [isAnimatingOut, setIsAnimatingOut] = useState(false);
  const snoozeButtonRef = useRef<HTMLButtonElement>(null);

  const handleComplete = () => {
    // Always let the parent handle completion logic
    // The parent (Intelligence.tsx) ALWAYS opens ChatModal for ALL tick actions
    onComplete(item);
  };

  const handleDismiss = () => {
    // Always let the parent handle dismiss logic and show confirmation modal
    // The parent (Intelligence.tsx) always shows confirmation for ALL dismiss actions
    onDismiss(item);
  };

  const handleSnooze = (duration: number) => {
    setIsAnimatingOut(true);
    setTimeout(() => {
      onSnooze(item, duration);
      setShowSnoozeMenu(false);
    }, 200);
  };

  // Priority styling
  const priorityClasses = {
    critical: 'border-coral-500/30 bg-coral-500/5',
    important: 'border-amber-500/30 bg-amber-500/5',
    normal: 'border-stone-200 bg-stone-50',
  };

  const priorityDotClasses = {
    critical: 'bg-coral-400',
    important: 'bg-amber-400',
    normal: 'bg-sage-400',
  };

  const sourceIcon = item.icon || SOURCE_ICONS[item.source];
  const isNew = isNewItem(item.createdAt);
  const timeAgo = formatTimeAgo(item.createdAt);

  return (
    <div
      className={`
        relative group transition-all duration-200 ease-in-out
        ${isAnimatingOut ? 'opacity-0 translate-x-4 scale-95' : 'animate-fade-up'}
        ${className}
      `}>
      <div
        className={`
          relative p-4 rounded-xl border backdrop-blur-sm transition-all duration-200
          hover:bg-stone-50 hover:border-stone-200
          ${priorityClasses[item.priority]}
        `}>
        {/* Main content row */}
        <div className="flex items-start gap-3">
          {/* Icon */}
          <div className="w-8 h-8 flex items-center justify-center text-stone-600 flex-shrink-0 mt-0.5">
            {sourceIcon}
          </div>

          {/* Content */}
          <div className="flex-1 min-w-0">
            <div className="flex items-start justify-between gap-3">
              <div className="flex-1 min-w-0">
                <h3 className="text-sm font-medium text-stone-900 leading-snug">{item.title}</h3>
                {item.description && (
                  <p className="text-xs text-stone-400 mt-1 leading-relaxed">{item.description}</p>
                )}
              </div>

              {/* Action buttons */}
              <div className="flex items-center gap-1 flex-shrink-0">
                {/* Complete button */}
                <button
                  onClick={handleComplete}
                  className="w-6 h-6 flex items-center justify-center rounded-md text-stone-400 hover:text-sage-400 hover:bg-sage-400/10 transition-all duration-150"
                  title="Complete">
                  <svg
                    className="w-3.5 h-3.5"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M5 13l4 4L19 7"
                    />
                  </svg>
                </button>

                {/* Dismiss button */}
                <button
                  onClick={handleDismiss}
                  className="w-6 h-6 flex items-center justify-center rounded-md text-stone-400 hover:text-coral-400 hover:bg-coral-400/10 transition-all duration-150"
                  title="Dismiss">
                  <svg
                    className="w-3.5 h-3.5"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M6 18L18 6M6 6l12 12"
                    />
                  </svg>
                </button>

                {/* Snooze button */}
                <div className="relative">
                  <button
                    ref={snoozeButtonRef}
                    onClick={() => setShowSnoozeMenu(!showSnoozeMenu)}
                    className="w-6 h-6 flex items-center justify-center rounded-md text-stone-400 hover:text-amber-400 hover:bg-amber-400/10 transition-all duration-150"
                    title="Snooze">
                    <svg
                      className="w-3.5 h-3.5"
                      fill="none"
                      stroke="currentColor"
                      viewBox="0 0 24 24">
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        strokeWidth={2}
                        d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"
                      />
                    </svg>
                  </button>
                </div>
              </div>
            </div>

            {/* Meta info */}
            <div className="flex items-center gap-2 mt-2">
              <div className="flex items-center gap-1.5">
                <div className={`w-1.5 h-1.5 rounded-full ${priorityDotClasses[item.priority]}`} />
                <span className="text-xs text-stone-500">{item.sourceLabel || item.source}</span>
              </div>
              <span className="text-xs text-stone-600">•</span>
              <span className="text-xs text-stone-500">{timeAgo}</span>
              {isNew && (
                <>
                  <span className="text-xs text-stone-600">•</span>
                  <span className="text-xs bg-sage-500 text-white px-1.5 py-0.5 rounded-sm font-medium">
                    New
                  </span>
                </>
              )}
            </div>
          </div>
        </div>
      </div>

      {/* Snooze dropdown portal - renders outside of any stacking context */}
      <SnoozeDropdownPortal
        isOpen={showSnoozeMenu}
        buttonRef={snoozeButtonRef}
        onClose={() => setShowSnoozeMenu(false)}
        onSnooze={handleSnooze}
      />
    </div>
  );
}
