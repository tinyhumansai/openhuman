import TunnelList from '../components/webhooks/TunnelList';
import WebhookActivity from '../components/webhooks/WebhookActivity';
import { useWebhooks } from '../hooks/useWebhooks';

export default function Webhooks() {
  const {
    tunnels,
    registrations,
    activity,
    loading,
    error,
    createTunnel,
    deleteTunnel,
    refreshTunnels,
  } = useWebhooks();

  if (loading && tunnels.length === 0) {
    return (
      <div className="flex flex-col h-full overflow-hidden">
        <div className="flex-1 flex items-center justify-center">
          <div className="flex flex-col items-center gap-3">
            <div className="h-8 w-8 animate-spin rounded-full border-2 border-stone-300 border-t-primary-500" />
            <span className="text-sm text-stone-500">Loading webhooks…</span>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full overflow-hidden">
      <div className="flex-1 overflow-y-auto p-6 space-y-8">
        {error && <div className="p-3 rounded-lg bg-coral-50 text-coral-700 text-sm">{error}</div>}

        <TunnelList
          tunnels={tunnels}
          registrations={registrations}
          loading={loading}
          onCreateTunnel={createTunnel}
          onDeleteTunnel={deleteTunnel}
          onRefresh={refreshTunnels}
        />

        <WebhookActivity activity={activity} />
      </div>
    </div>
  );
}
