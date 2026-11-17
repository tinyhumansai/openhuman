import { createContext, type PropsWithChildren, useContext, useMemo, useState } from 'react';

export const TaskCardDockTarget = createContext<HTMLElement | null>(null);
const SetDockTarget = createContext<(element: HTMLDivElement | null) => void>(() => {});

/** Active task cards share a transcript dock and return to their turn on completion. */
export function TaskCardDockProvider({ children }: PropsWithChildren) {
  const [target, setTarget] = useState<HTMLDivElement | null>(null);
  const setter = useMemo(() => setTarget, []);
  return (
    <TaskCardDockTarget.Provider value={target}>
      <SetDockTarget.Provider value={setter}>{children}</SetDockTarget.Provider>
    </TaskCardDockTarget.Provider>
  );
}
export function TaskCardDock({ children }: PropsWithChildren) {
  const setTarget = useContext(SetDockTarget);
  return (
    <div
      ref={setTarget}
      data-slot="task-card-dock"
      className="flex flex-col gap-3 py-3 empty:hidden [&>[data-slot=task-card]]:max-w-none">
      {children}
    </div>
  );
}
