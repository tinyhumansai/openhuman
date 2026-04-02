import { useEffect, useRef, useState } from 'react';

import { useAppDispatch, useAppSelector } from '../../store/hooks';
import { resetDictation } from '../../store/dictationSlice';
import { openhumanAccessibilityInputAction } from '../../utils/tauriCommands';
import { useDictation } from './useDictation';

const STATUS_COLORS: Record<string, string> = {
  idle: 'bg-stone-800',
  recording: 'bg-red-600 animate-pulse',
  transcribing: 'bg-amber-600',
  ready: 'bg-primary-600',
  error: 'bg-coral-600',
};

const STATUS_LABELS: Record<string, string> = {
  idle: 'Idle',
  recording: 'Recording...',
  transcribing: 'Transcribing...',
  ready: 'Ready',
  error: 'Error',
};

const DictationOverlay = () => {
  const dispatch = useAppDispatch();
  const { status, transcript, error, hotkey } = useAppSelector(s => s.dictation);
  const { startRecording, stopRecording, dismiss } = useDictation();
  const panelRef = useRef<HTMLDivElement | null>(null);
  const dragRef = useRef<{ pointerId: number; offsetX: number; offsetY: number } | null>(null);
  const prevStatusRef = useRef(status);
  const [position, setPosition] = useState<{ x: number; y: number } | null>(null);

  const getActiveDefaultPosition = () => {
    if (typeof window === 'undefined') return { x: 24, y: 24 };
    return {
      x: Math.max(12, Math.round(window.innerWidth / 2 - 200)),
      y: Math.max(12, window.innerHeight - 240),
    };
  };

  const getIdleDefaultPosition = () => {
    if (typeof window === 'undefined') return { x: 24, y: 24 };
    const idleWidth = 250;
    const idleHeight = 62;
    return {
      x: Math.max(12, window.innerWidth - idleWidth - 24),
      y: Math.max(12, window.innerHeight - idleHeight - 24),
    };
  };

  const clampToViewport = (x: number, y: number) => {
    const panelWidth = panelRef.current?.offsetWidth ?? 360;
    const panelHeight = panelRef.current?.offsetHeight ?? 220;
    const maxX = Math.max(12, window.innerWidth - panelWidth - 12);
    const maxY = Math.max(12, window.innerHeight - panelHeight - 12);
    return {
      x: Math.max(12, Math.min(x, maxX)),
      y: Math.max(12, Math.min(y, maxY)),
    };
  };

  // Keyboard shortcut: Escape to dismiss
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        console.debug('[dictation] Escape pressed — dismissing overlay');
        dismiss();
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [dismiss]);

  useEffect(() => {
    if (position === null) {
      setPosition(status === 'idle' ? getIdleDefaultPosition() : getActiveDefaultPosition());
    }
  }, [position, status]);

  useEffect(() => {
    const prev = prevStatusRef.current;
    if (status === 'idle' && prev !== 'idle') {
      // When closing active overlay, reset launcher to the corner.
      setPosition(getIdleDefaultPosition());
    }
    prevStatusRef.current = status;
  }, [status]);

  useEffect(() => {
    const onResize = () => {
      setPosition(prev => {
        const base =
          prev ?? (status === 'idle' ? getIdleDefaultPosition() : getActiveDefaultPosition());
        return clampToViewport(base.x, base.y);
      });
    };
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, [status]);

  const handleDragStart = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest('button')) return;
    if (!panelRef.current) return;
    const rect = panelRef.current.getBoundingClientRect();
    dragRef.current = {
      pointerId: e.pointerId,
      offsetX: e.clientX - rect.left,
      offsetY: e.clientY - rect.top,
    };
    e.currentTarget.setPointerCapture(e.pointerId);
    setPosition({ x: rect.left, y: rect.top });
  };

  const handleDragMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== e.pointerId) return;
    e.preventDefault();
    const next = clampToViewport(e.clientX - drag.offsetX, e.clientY - drag.offsetY);
    setPosition(next);
  };

  const handleDragEnd = (e: React.PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== e.pointerId) return;
    dragRef.current = null;
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId);
    }
  };

  const handleInsert = async () => {
    if (!transcript) return;
    console.debug('[dictation] inserting transcript via accessibility action');
    try {
      await openhumanAccessibilityInputAction({ action: 'type', text: transcript });
      dispatch(resetDictation());
    } catch {
      // Fallback to clipboard
      console.debug('[dictation] accessibility insert failed, falling back to clipboard');
      await navigator.clipboard.writeText(transcript).catch(() => {});
      dispatch(resetDictation());
    }
  };

  const handleCopy = async () => {
    if (!transcript) return;
    console.debug('[dictation] copying transcript to clipboard');
    await navigator.clipboard.writeText(transcript).catch(() => {});
    dispatch(resetDictation());
  };

  if (status === 'idle') {
    return (
      <div
        className="fixed z-50 pointer-events-auto"
        style={{ left: position?.x ?? 24, top: position?.y ?? 24 }}>
        <div
          ref={panelRef}
          className="rounded-full bg-stone-900/95 border border-stone-700/60 shadow-xl p-1 backdrop-blur-md flex items-center gap-1.5">
          <div
            className="h-9 w-9 rounded-full bg-stone-800/80 border border-stone-700 flex items-center justify-center text-stone-300 cursor-grab active:cursor-grabbing select-none touch-none"
            title="Drag"
            onPointerDown={handleDragStart}
            onPointerMove={handleDragMove}
            onPointerUp={handleDragEnd}
            onPointerCancel={handleDragEnd}>
            ⋮⋮
          </div>
          <button
            onClick={() => void startRecording()}
            className="flex items-center gap-2 px-4 py-2 rounded-full bg-red-600 hover:bg-red-500 text-white text-sm font-medium transition-colors"
            aria-label="Start dictation">
            <span className="w-2.5 h-2.5 rounded-full bg-white" />
            Start Dictation
          </button>
        </div>
      </div>
    );
  }

  return (
    <div
      className="fixed z-50 pointer-events-auto"
      style={{ left: position?.x ?? 24, top: position?.y ?? 24 }}>
      <div
        ref={panelRef}
        className="bg-stone-900/95 backdrop-blur-md border border-stone-700/60 rounded-2xl shadow-2xl p-4 min-w-[320px] max-w-[480px]">
        {/* Header */}
        <div
          className="flex items-center gap-3 mb-3 cursor-move select-none touch-none"
          onPointerDown={handleDragStart}
          onPointerMove={handleDragMove}
          onPointerUp={handleDragEnd}
          onPointerCancel={handleDragEnd}>
          <div className={`w-2.5 h-2.5 rounded-full flex-shrink-0 ${STATUS_COLORS[status] ?? 'bg-stone-800'}`} />
          <span className="text-sm font-medium text-white">
            {STATUS_LABELS[status] ?? 'Dictation'}
          </span>
          <div className="flex-1" />
          <button
            onClick={dismiss}
            className="text-stone-400 hover:text-stone-200 transition-colors text-xs"
            aria-label="Dismiss dictation">
            ✕
          </button>
        </div>

        {/* Transcript */}
        {transcript && (
          <div className="mb-3 p-3 bg-stone-800/60 rounded-lg border border-stone-700/40">
            <p className="text-sm text-stone-100 leading-relaxed">{transcript}</p>
          </div>
        )}

        {/* Error */}
        {error && (
          <div className="mb-3 p-2 bg-red-500/10 border border-red-500/20 rounded-lg">
            <p className="text-xs text-red-400">{error}</p>
          </div>
        )}

        {/* Controls */}
        <div className="flex items-center gap-2">
          {(status === 'error' || status === 'ready') && (
            <button
              onClick={() => void startRecording()}
              className="flex items-center gap-1.5 px-3 py-1.5 bg-red-600 hover:bg-red-500 text-white text-xs font-medium rounded-lg transition-colors">
              <span className="w-2 h-2 rounded-full bg-white" />
              Record
            </button>
          )}

          {status === 'recording' && (
            <button
              onClick={stopRecording}
              className="flex items-center gap-1.5 px-3 py-1.5 bg-stone-700 hover:bg-stone-600 text-white text-xs font-medium rounded-lg transition-colors">
              <span className="w-2 h-2 bg-white rounded-sm" />
              Stop
            </button>
          )}

          {transcript && (
            <>
              <button
                onClick={() => void handleInsert()}
                className="flex-1 px-3 py-1.5 bg-primary-600 hover:bg-primary-500 text-white text-xs font-medium rounded-lg transition-colors">
                Insert
              </button>
              <button
                onClick={() => void handleCopy()}
                className="px-3 py-1.5 bg-stone-700 hover:bg-stone-600 text-stone-200 text-xs font-medium rounded-lg transition-colors">
                Copy
              </button>
            </>
          )}
        </div>

        {/* Hotkey hint */}
        <div className="mt-2 text-[10px] text-stone-500 text-center">
          {hotkey} to toggle &middot; Esc to dismiss
        </div>
      </div>
    </div>
  );
};

export default DictationOverlay;
