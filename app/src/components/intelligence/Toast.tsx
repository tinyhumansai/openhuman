import { useEffect, useRef } from 'react';

import type { ToastNotification } from '../../types/intelligence';
import { toast } from '../ui/Toast';

interface ToastContainerProps {
  notifications: ToastNotification[];
  onRemove: (id: string) => void;
}

/** Bridges legacy page toast state into the single app-wide toast manager. */
export function ToastContainer({ notifications, onRemove }: ToastContainerProps) {
  const dispatched = useRef(new Set<string>());

  useEffect(() => {
    for (const notification of notifications) {
      if (dispatched.current.has(notification.id)) continue;
      dispatched.current.add(notification.id);
      toast.add({
        type: notification.type,
        title: notification.title,
        description: notification.message,
        ...(notification.action
          ? {
              actionProps: {
                children: notification.action.label,
                onClick: notification.action.handler,
              },
            }
          : {}),
      });
      onRemove(notification.id);
    }
  }, [notifications, onRemove]);

  return null;
}
