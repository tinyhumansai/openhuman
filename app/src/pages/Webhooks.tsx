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
    coreConnected,
    createTunnel,
    deleteTunnel,
    refreshTunnels,
    registerEcho,
    unregisterEcho,
  } = useWebhooks();

  if (loading && tunnels.length === 0) {
    return (
      <div className="h-full flex items-center justify-center p-4 pt-6">
        <div className="flex flex-col items-center gap-3">
          <div className="h-8 w-8 animate-spin rounded-full border-2 border-stone-300 border-t-primary-500" />
          <span className="text-sm text-stone-500">Loading webhooks…</span>
        </div>
      </div>
    );
  }

  return (
    <div className="h-full overflow-y-auto p-4 pt-6">
      <div className="max-w-2xl mx-auto space-y-4">
        {/* Connection status */}
        <div className="flex items-center gap-3">
          <h2 className="text-xl font-semibold text-stone-900">Webhooks</h2>
          <span
            className={`inline-flex items-center gap-1.5 px-2.5 py-1 text-xs font-medium rounded-full ${
              coreConnected ? 'bg-sage-100 text-sage-700' : 'bg-stone-100 text-stone-500'
            }`}>
            <span
              className={`w-1.5 h-1.5 rounded-full ${
                coreConnected ? 'bg-sage-500' : 'bg-stone-400'
              }`}
            />
            {coreConnected ? 'Connected' : 'Disconnected'}
          </span>
        </div>

        {error && <div className="p-3 rounded-lg bg-coral-50 text-coral-700 text-sm">{error}</div>}

        <div className="bg-white rounded-2xl shadow-soft border border-stone-200 p-6">
          <TunnelList
            tunnels={tunnels}
            registrations={registrations}
            loading={loading}
            onCreateTunnel={createTunnel}
            onDeleteTunnel={deleteTunnel}
            onRefresh={refreshTunnels}
            onRegisterEcho={registerEcho}
            onUnregisterEcho={unregisterEcho}
          />
        </div>

        <div className="bg-white rounded-2xl shadow-soft border border-stone-200 p-6">
          <WebhookActivity activity={activity} />
        </div>
      </div>
    </div>
  );
}
