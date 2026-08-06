import {
  type PointerEvent as ReactPointerEvent,
  type RefObject,
  useCallback,
  useEffect,
  useRef,
} from 'react';

import { useEscapeKey } from './useEscapeKey';

export interface UseDismissLayerOptions {
  onDismiss: () => void;
  enabled?: boolean;
  dismissOnEscape?: boolean;
  dismissOnOutsidePointer?: boolean;
}

export interface DismissLayerBindings {
  layerRef: RefObject<HTMLElement | null>;
  onPointerDownCapture: (event: ReactPointerEvent) => void;
}

export function useDismissLayer({
  onDismiss,
  enabled = true,
  dismissOnEscape = true,
  dismissOnOutsidePointer = true,
}: UseDismissLayerOptions): DismissLayerBindings {
  const layerRef = useRef<HTMLElement>(null);
  const onDismissRef = useRef(onDismiss);
  const mountedRef = useRef(false);
  onDismissRef.current = onDismiss;

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const dismiss = useCallback(() => {
    if (mountedRef.current) onDismissRef.current();
  }, []);

  useEscapeKey(dismiss, enabled && dismissOnEscape);

  const onPointerDownCapture = useCallback(
    (event: ReactPointerEvent) => {
      if (!mountedRef.current || !enabled || !dismissOnOutsidePointer) return;

      const layer = layerRef.current;
      const target = event.target;
      if (!layer || !(target instanceof Node) || layer.contains(target)) return;

      onDismissRef.current();
    },
    [dismissOnOutsidePointer, enabled]
  );

  return { layerRef, onPointerDownCapture };
}
