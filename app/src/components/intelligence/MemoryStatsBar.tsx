interface MemoryStatsBarProps {
  totalDocs: number;
  totalFiles: number;
  totalNamespaces: number;
  totalRelations: number;
  totalSessions: number | null;
  totalTokens: number | null;
  /** Estimated storage in bytes (sum of document content lengths). */
  estimatedStorageBytes: number;
  /** Unix-epoch seconds of the oldest document. */
  oldestDocTimestamp: number | null;
  /** Unix-epoch seconds of the newest document. */
  newestDocTimestamp: number | null;
  docsToday: number;
  loading?: boolean;
}

function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB'];
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / Math.pow(1024, i);
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[i]}`;
}

function formatTimeAgo(epochSeconds: number): string {
  const now = Date.now() / 1000;
  const diff = now - epochSeconds;
  if (diff < 60) return 'just now';
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  if (diff < 2592000) return `${Math.floor(diff / 86400)}d ago`;
  if (diff < 31536000) return `${Math.floor(diff / 2592000)}mo ago`;
  return `${(diff / 31536000).toFixed(1)}y ago`;
}

function formatNumber(value: number): string {
  return new Intl.NumberFormat().format(value);
}

export function MemoryStatsBar(props: MemoryStatsBarProps) {
  const {
    totalDocs,
    totalFiles,
    totalNamespaces,
    totalRelations,
    totalSessions,
    totalTokens,
    estimatedStorageBytes,
    oldestDocTimestamp,
    newestDocTimestamp,
    docsToday,
    loading,
  } = props;

  const stats = [
    {
      label: 'Storage',
      value: estimatedStorageBytes > 0 ? formatBytes(estimatedStorageBytes) : '--',
      sub: totalFiles > 0 ? `${formatNumber(totalFiles)} files` : undefined,
      color: 'text-primary-300',
    },
    {
      label: 'Documents',
      value: formatNumber(totalDocs),
      sub: docsToday > 0 ? `+${docsToday} today` : undefined,
      color: 'text-emerald-300',
    },
    {
      label: 'Namespaces',
      value: formatNumber(totalNamespaces),
      sub: undefined,
      color: 'text-amber-300',
    },
    {
      label: 'Relations',
      value: formatNumber(totalRelations),
      sub: undefined,
      color: 'text-lavender-300',
    },
    {
      label: 'First Memory',
      value: oldestDocTimestamp ? formatTimeAgo(oldestDocTimestamp) : '--',
      sub: newestDocTimestamp ? `Latest: ${formatTimeAgo(newestDocTimestamp)}` : undefined,
      color: 'text-sky-300',
    },
    {
      label: 'Sessions',
      value: totalSessions !== null ? formatNumber(totalSessions) : '--',
      sub: totalTokens !== null ? `${formatNumber(totalTokens)} tokens` : undefined,
      color: 'text-rose-300',
    },
  ];

  return (
    <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-6 gap-3">
      {stats.map(stat => (
        <div
          key={stat.label}
          className="rounded-xl border border-white/10 bg-black/20 p-3 transition-colors hover:bg-black/30">
          <div className="text-[11px] uppercase tracking-wide text-stone-500 mb-1">
            {stat.label}
          </div>
          <div className={`text-xl font-semibold ${stat.color}`}>
            {loading ? <div className="h-7 w-16 rounded bg-white/5 animate-pulse" /> : stat.value}
          </div>
          {stat.sub && <div className="text-[11px] text-stone-500 mt-0.5">{stat.sub}</div>}
        </div>
      ))}
    </div>
  );
}
