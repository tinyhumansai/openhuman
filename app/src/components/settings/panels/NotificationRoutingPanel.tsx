import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

/**
 * Settings panel for the notification intelligence / routing pipeline.
 *
 * Currently exposes a global explanation card. Per-provider threshold
 * controls will populate here as providers are connected.
 */
const NotificationRoutingPanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();

  return (
    <div>
      <SettingsHeader
        title="Notification Routing"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-4">
        {/* Info card */}
        <div className="p-4 bg-blue-50 border border-blue-200 rounded-xl">
          <div className="flex items-start space-x-3">
            <svg
              className="w-5 h-5 text-blue-600 flex-shrink-0 mt-0.5"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z"
              />
            </svg>
            <div>
              <p className="font-medium text-blue-800 text-sm">Notification Intelligence</p>
              <p className="text-blue-700 text-xs mt-1 leading-relaxed">
                Every notification from your connected accounts is scored by a local AI model.
                High-importance notifications are automatically routed to your orchestrator agent so
                nothing critical slips through.
              </p>
            </div>
          </div>
        </div>

        {/* How it works */}
        <div className="bg-white border border-stone-200 rounded-xl overflow-hidden">
          <div className="px-4 py-3 border-b border-stone-100">
            <p className="text-sm font-medium text-stone-900">How it works</p>
          </div>
          <div className="divide-y divide-stone-100">
            {[
              {
                label: 'Drop',
                desc: 'Noise / spam — stored but not surfaced',
                color: 'bg-stone-100 text-stone-600',
              },
              {
                label: 'Acknowledge',
                desc: 'Low-priority — shown in notification center',
                color: 'bg-blue-100 text-blue-700',
              },
              {
                label: 'React',
                desc: 'Medium-priority — triggers a focused agent response',
                color: 'bg-amber-100 text-amber-700',
              },
              {
                label: 'Escalate',
                desc: 'High-priority — forwarded to orchestrator agent',
                color: 'bg-red-100 text-red-700',
              },
            ].map(row => (
              <div key={row.label} className="flex items-center gap-3 px-4 py-3">
                <span
                  className={`flex-shrink-0 px-2 py-0.5 rounded text-[11px] font-semibold ${row.color}`}>
                  {row.label}
                </span>
                <span className="text-xs text-stone-600">{row.desc}</span>
              </div>
            ))}
          </div>
        </div>

        {/* Placeholder for per-provider settings */}
        <div className="p-4 bg-stone-50 border border-stone-200 rounded-xl text-center">
          <p className="text-xs text-stone-500">
            Per-provider importance thresholds will appear here once you connect accounts via the
            Accounts screen.
          </p>
        </div>
      </div>
    </div>
  );
};

export default NotificationRoutingPanel;
