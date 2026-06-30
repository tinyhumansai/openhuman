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
      <div className="md:col-span-2 rounded-2xl border border-line p-5 h-32 bg-surface-muted" />
      <div className="grid grid-cols-2 md:grid-cols-1 gap-3">
        <div className="rounded-2xl border border-line h-14 bg-surface-muted" />
        <div className="rounded-2xl border border-line h-14 bg-surface-muted" />
      </div>
    </div>
    <div className="rounded-2xl border border-line p-4 h-64 bg-surface-muted" />
    <div className="rounded-2xl border border-line p-4 h-64 bg-surface-muted" />
    <div className="rounded-2xl border border-line p-4 h-32 bg-surface-muted" />
  </div>
);

export default DashboardSkeleton;
