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
      <div className="h-full bg-[#F5F5F5] flex items-center justify-center p-4 pt-6">
        <div className="flex flex-col items-center gap-3">
          <div className="h-8 w-8 animate-spin rounded-full border-2 border-stone-300 border-t-primary-500" />
          <span className="text-sm text-stone-500">Loading webhooks…</span>
        </div>
      </div>
    );
  }

  return (
    <div className="h-full bg-[#F5F5F5] overflow-y-auto p-4 pt-6">
      <div className="max-w-2xl mx-auto space-y-4">
        {error && <div className="p-3 rounded-lg bg-coral-50 text-coral-700 text-sm">{error}</div>}

        <div className="bg-white rounded-2xl shadow-soft border border-stone-200 p-6">
          <TunnelList
            tunnels={tunnels}
            registrations={registrations}
            loading={loading}
            onCreateTunnel={createTunnel}
            onDeleteTunnel={deleteTunnel}
            onRefresh={refreshTunnels}
          />
        </div>

        <div className="bg-white rounded-2xl shadow-soft border border-stone-200 p-6">
          <WebhookActivity activity={activity} />
        </div>
      </div>
    </div>
  );
}
