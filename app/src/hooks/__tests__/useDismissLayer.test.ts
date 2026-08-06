import { fireEvent, render, renderHook, screen } from '@testing-library/react';
import {
  createElement,
  type ReactNode,
  type PointerEvent as ReactPointerEvent,
  StrictMode,
} from 'react';
import { createPortal } from 'react-dom';
import { describe, expect, test, vi } from 'vitest';

import { useDismissLayer, type UseDismissLayerOptions } from '../useDismissLayer';

function DismissLayerHarness({
  options,
  portal,
  renderLayer = true,
}: {
  options: UseDismissLayerOptions;
  portal?: ReactNode;
  renderLayer?: boolean;
}) {
  const bindings = useDismissLayer(options);

  return createElement(
    'div',
    { 'data-testid': 'capture-root', onPointerDownCapture: bindings.onPointerDownCapture },
    renderLayer
      ? createElement(
          'div',
          { ref: bindings.layerRef, 'data-testid': 'layer' },
          createElement('button', { type: 'button' }, 'Inside child')
        )
      : null,
    createElement('button', { type: 'button' }, 'Outside sibling'),
    portal ? createPortal(portal, document.body) : null
  );
}

describe('useDismissLayer', () => {
  test('dismisses only pointer targets outside the current layer', () => {
    const onDismiss = vi.fn();
    render(createElement(DismissLayerHarness, { options: { onDismiss } }));

    fireEvent.pointerDown(screen.getByRole('button', { name: 'Inside child' }));
    expect(onDismiss).not.toHaveBeenCalled();

    fireEvent.pointerDown(screen.getByRole('button', { name: 'Outside sibling' }));
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  test('applies enabled, Escape, and outside-pointer policies independently', () => {
    const onDismiss = vi.fn();
    const { rerender } = render(
      createElement(DismissLayerHarness, {
        options: { onDismiss, dismissOnEscape: false, dismissOnOutsidePointer: true },
      })
    );

    fireEvent.keyDown(document, { key: 'Escape' });
    fireEvent.pointerDown(screen.getByRole('button', { name: 'Outside sibling' }));
    expect(onDismiss).toHaveBeenCalledTimes(1);

    rerender(
      createElement(DismissLayerHarness, {
        options: { onDismiss, dismissOnEscape: true, dismissOnOutsidePointer: false },
      })
    );
    fireEvent.pointerDown(screen.getByRole('button', { name: 'Outside sibling' }));
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onDismiss).toHaveBeenCalledTimes(2);

    rerender(
      createElement(DismissLayerHarness, {
        options: {
          onDismiss,
          enabled: false,
          dismissOnEscape: true,
          dismissOnOutsidePointer: true,
        },
      })
    );
    fireEvent.pointerDown(screen.getByRole('button', { name: 'Outside sibling' }));
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onDismiss).toHaveBeenCalledTimes(2);
  });

  test('uses the latest callback without re-registering the Escape listener', () => {
    const addEventListener = vi.spyOn(document, 'addEventListener');
    const removeEventListener = vi.spyOn(document, 'removeEventListener');
    const first = vi.fn();
    const second = vi.fn();

    const { rerender, unmount } = renderHook(({ onDismiss }) => useDismissLayer({ onDismiss }), {
      initialProps: { onDismiss: first },
    });
    const keydownAddsBefore = addEventListener.mock.calls.filter(
      ([type]) => type === 'keydown'
    ).length;
    const keydownRemovesBefore = removeEventListener.mock.calls.filter(
      ([type]) => type === 'keydown'
    ).length;

    rerender({ onDismiss: second });
    fireEvent.keyDown(document, { key: 'Escape' });

    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledTimes(1);
    expect(addEventListener.mock.calls.filter(([type]) => type === 'keydown')).toHaveLength(
      keydownAddsBefore
    );
    expect(removeEventListener.mock.calls.filter(([type]) => type === 'keydown')).toHaveLength(
      keydownRemovesBefore
    );

    unmount();
    addEventListener.mockRestore();
    removeEventListener.mockRestore();
  });

  test('does nothing after unmount', () => {
    const onDismiss = vi.fn();
    const { result, unmount } = renderHook(() => useDismissLayer({ onDismiss }));
    const retainedPointerHandler = result.current.onPointerDownCapture;

    unmount();
    fireEvent.keyDown(document, { key: 'Escape' });
    retainedPointerHandler({ target: document.body } as unknown as ReactPointerEvent);

    expect(onDismiss).not.toHaveBeenCalled();
  });

  test('dismisses exactly once per event in StrictMode', () => {
    const onDismiss = vi.fn();
    render(
      createElement(
        StrictMode,
        null,
        createElement(DismissLayerHarness, { options: { onDismiss } })
      )
    );

    fireEvent.pointerDown(screen.getByRole('button', { name: 'Outside sibling' }));
    fireEvent.keyDown(document, { key: 'Escape' });

    expect(onDismiss).toHaveBeenCalledTimes(2);
  });

  test('does not dismiss from pointer input while the layer ref is null', () => {
    const onDismiss = vi.fn();
    render(createElement(DismissLayerHarness, { options: { onDismiss }, renderLayer: false }));

    fireEvent.pointerDown(screen.getByRole('button', { name: 'Outside sibling' }));

    expect(onDismiss).not.toHaveBeenCalled();
  });

  test('treats a portal target outside the layer DOM as an outside pointer', () => {
    const onDismiss = vi.fn();
    render(
      createElement(DismissLayerHarness, {
        options: { onDismiss },
        portal: createElement('button', { type: 'button' }, 'Portal action'),
      })
    );

    fireEvent.pointerDown(screen.getByRole('button', { name: 'Portal action' }));

    expect(onDismiss).toHaveBeenCalledTimes(1);
  });
});
