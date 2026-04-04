import type { ChannelDefinition } from '../../types/channels';
import ChannelStatusBadge from './ChannelStatusBadge';

interface WebChannelConfigProps {
  definition: ChannelDefinition;
}

const WebChannelConfig = ({ definition }: WebChannelConfigProps) => {
  return (
    <section className="rounded-xl border border-stone-800/60 bg-black/40 p-4 space-y-3">
      <div className="flex items-start justify-between">
        <div>
          <h3 className="text-base font-semibold text-white">{definition.display_name}</h3>
          <p className="text-xs text-stone-400 mt-1">{definition.description}</p>
        </div>
        <ChannelStatusBadge status="connected" />
      </div>
      <p className="text-sm text-stone-300">
        The web channel is always available — no setup required.
      </p>
    </section>
  );
};

export default WebChannelConfig;
