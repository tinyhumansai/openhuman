/**
 * Loading skeleton for the cost dashboard panel. Renders the same overall
 * layout as the populated dashboard so the first paint doesn't reflow
 * dramatically once data arrives.
 */
const DashboardSkeleton = () => (
  <div
    role="status"
    aria-live="polite"
    data-testid="cost-dashboard-skeleton"
    className="space-y-4 animate-pulse">
    <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
      <div className="md:col-span-2 rounded-2xl border border-stone-200 dark:border-neutral-800 p-5 h-32 bg-stone-50 dark:bg-neutral-900/40" />
      <div className="grid grid-cols-2 md:grid-cols-1 gap-3">
        <div className="rounded-2xl border border-stone-200 dark:border-neutral-800 h-14 bg-stone-50 dark:bg-neutral-900/40" />
        <div className="rounded-2xl border border-stone-200 dark:border-neutral-800 h-14 bg-stone-50 dark:bg-neutral-900/40" />
      </div>
    </div>
    <div className="rounded-2xl border border-stone-200 dark:border-neutral-800 p-4 h-64 bg-stone-50 dark:bg-neutral-900/40" />
    <div className="rounded-2xl border border-stone-200 dark:border-neutral-800 p-4 h-64 bg-stone-50 dark:bg-neutral-900/40" />
    <div className="rounded-2xl border border-stone-200 dark:border-neutral-800 p-4 h-32 bg-stone-50 dark:bg-neutral-900/40" />
  </div>
);

export default DashboardSkeleton;
