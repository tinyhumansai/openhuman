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

  return (
    <div className="flex flex-col h-full overflow-hidden">
      <div className="flex-1 overflow-y-auto p-6 space-y-8">
        {error && (
          <div className="p-3 rounded-lg bg-coral-50 text-coral-700 text-sm">
            {error}
          </div>
        )}

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
