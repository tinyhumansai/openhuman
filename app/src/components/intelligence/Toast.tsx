import { useCallback, useEffect, useState } from 'react';

import type { ToastNotification } from '../../types/intelligence';

interface ToastProps {
  notification: ToastNotification;
  onRemove: (id: string) => void;
}

const TOAST_ICONS = {
  success: (
    <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
    </svg>
  ),
  error: (
    <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
    </svg>
  ),
  warning: (
    <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-2.5L13.732 4c-.77-.833-1.964-.833-2.732 0L3.732 16.5c-.77.833.192 2.5 1.732 2.5z"
      />
    </svg>
  ),
  info: (
    <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
      />
    </svg>
  ),
};

const TOAST_STYLES = {
  success: 'bg-neutral-0 border-sage-500 text-content',
  error: 'bg-neutral-0 border-coral-500 text-content',
  warning: 'bg-neutral-0 border-amber-500 text-content',
  info: 'bg-neutral-0 border-primary-500 text-content',
};

const TOAST_ICON_STYLES = {
  success: 'text-sage-600',
  error: 'text-coral-500',
  warning: 'text-amber-600',
  info: 'text-primary-500',
};

export function Toast({ notification, onRemove }: ToastProps) {
  const [isVisible, setIsVisible] = useState(false);
  const [isExiting, setIsExiting] = useState(false);

  const handleRemove = useCallback(() => {
    setIsExiting(true);
    setTimeout(() => {
      onRemove(notification.id);
    }, 200);
  }, [onRemove, notification.id]);

  useEffect(() => {
    // Animate in
    const showTimer = setTimeout(() => setIsVisible(true), 50);

    // Auto remove after duration
    const duration = notification.duration || 4000;
    const removeTimer = setTimeout(() => {
      handleRemove();
    }, duration);

    return () => {
      clearTimeout(showTimer);
      clearTimeout(removeTimer);
    };
  }, [notification, handleRemove]);

  const icon = TOAST_ICONS[notification.type];
  const styles = TOAST_STYLES[notification.type];
  const iconStyle = TOAST_ICON_STYLES[notification.type];

  return (
    <div
      className={`
        transform transition-all duration-200 ease-in-out
        ${isVisible && !isExiting ? 'translate-x-0 opacity-100' : 'translate-x-full opacity-0'}
        ${isExiting ? 'scale-95' : ''}
      `}>
      <div
        className={`
          flex items-center gap-3 p-4 rounded-lg shadow-large border-l-4 border backdrop-blur-sm
          max-w-sm w-full
          ${styles}
        `}>
        {/* Icon */}
        <div className={`flex-shrink-0 ${iconStyle}`}>{icon}</div>

        {/* Content */}
        <div className="flex-1 min-w-0">
          <h4 className="text-sm font-medium">{notification.title}</h4>
          {notification.message && (
            <p className="text-xs text-content-muted mt-1">{notification.message}</p>
          )}
        </div>

        {/* Action button */}
        {notification.action && (
          <button
            onClick={notification.action.handler}
            className="text-xs font-medium underline hover:no-underline flex-shrink-0">
            {notification.action.label}
          </button>
        )}

        {/* Close button */}
        <button
          onClick={handleRemove}
          className="flex-shrink-0 text-content-faint hover:text-content-secondary transition-colors">
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M6 18L18 6M6 6l12 12"
            />
          </svg>
        </button>
      </div>
    </div>
  );
}

interface ToastContainerProps {
  notifications: ToastNotification[];
  onRemove: (id: string) => void;
}

export function ToastContainer({ notifications, onRemove }: ToastContainerProps) {
  if (notifications.length === 0) return null;

  return (
    <div className="fixed top-4 right-4 z-50 space-y-2 pointer-events-none">
      <div className="pointer-events-auto">
        {notifications.map(notification => (
          <Toast key={notification.id} notification={notification} onRemove={onRemove} />
        ))}
      </div>
    </div>
  );
}
