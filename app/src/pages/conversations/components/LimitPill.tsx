export function LimitPill({ label, usedPct }: { label: string; usedPct: number }) {
  const barColor =
    usedPct >= 1 ? 'bg-coral-500' : usedPct >= 0.8 ? 'bg-amber-500' : 'bg-primary-500';
  return (
    <div className="flex items-center gap-1">
      <span className="text-[9px] text-stone-400 font-medium uppercase">{label}</span>
      <div className="w-8 h-1.5 rounded-full bg-stone-200 overflow-hidden">
        <div
          className={`h-full rounded-full transition-all duration-300 ${barColor}`}
          style={{ width: `${Math.min(100, usedPct * 100)}%` }}
        />
      </div>
      <span className="text-[9px] text-stone-500 tabular-nums">{Math.round(usedPct * 100)}%</span>
    </div>
  );
}
