interface ChannelCapabilitiesProps {
  capabilities: string[];
}

const ChannelCapabilities = ({ capabilities }: ChannelCapabilitiesProps) => {
  if (capabilities.length === 0) return null;
  return (
    <div className="flex flex-wrap gap-1.5 mt-2">
      {capabilities.map(cap => (
        <span
          key={cap}
          className="px-1.5 py-0.5 text-[10px] rounded bg-surface-subtle text-content-muted border border-line">
          {cap.replace(/_/g, ' ')}
        </span>
      ))}
    </div>
  );
};

export default ChannelCapabilities;
