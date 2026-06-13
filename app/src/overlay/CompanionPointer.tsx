/**
 * CompanionPointer — renders point-target labels inside the overlay bubble area.
 */
import { useEffect, useState } from 'react';

export interface PointTarget {
  absolute_x: number;
  absolute_y: number;
  label: string;
}

interface CompanionPointerProps {
  targets: PointTarget[];
  /** Auto-dismiss each target after this many ms. Default: 2000 */
  dismissMs?: number;
}

export default function CompanionPointer({ targets, dismissMs = 2000 }: CompanionPointerProps) {
  const [visible, setVisible] = useState(true);

  useEffect(() => {
    if (targets.length === 0) return;
    const showFrame = window.requestAnimationFrame(() => setVisible(true));
    const dismissTimer = window.setTimeout(() => setVisible(false), dismissMs);
    return () => {
      window.cancelAnimationFrame(showFrame);
      window.clearTimeout(dismissTimer);
    };
  }, [targets, dismissMs]);

  if (!visible || targets.length === 0) return null;

  return (
    <div className="flex flex-col items-end gap-1">
      {targets.map((t, i) => (
        <div
          key={`${t.label}-${i}`}
          className="animate-[overlay-bubble-in_220ms_ease-out] rounded-lg bg-blue-600 px-2 py-1 text-xs text-white shadow-md">
          <span className="mr-1 inline-block h-2 w-2 rounded-full bg-blue-300 animate-pulse" />
          {t.label}
        </div>
      ))}
    </div>
  );
}
