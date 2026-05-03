import { Ghosty } from './Mascot';

const HumanPage = () => {
  return (
    <div className="absolute inset-0 flex bg-stone-100">
      {/* Mascot stage */}
      <div className="relative flex-1 flex items-center justify-center overflow-hidden">
        <div
          className="pointer-events-none absolute inset-0"
          style={{
            background:
              'radial-gradient(ellipse at 50% 40%, rgba(74,131,221,0.10), transparent 60%)',
          }}
        />
        <div className="relative w-[min(80vh,80vw)] aspect-square">
          <Ghosty />
        </div>
      </div>

      {/* Thread panel — embedded Conversations lands in phase 3. */}
      <aside className="w-[420px] border-l border-stone-300 bg-white flex flex-col">
        <div className="px-4 py-3 border-b border-stone-200">
          <h2 className="text-sm font-semibold text-stone-900">Conversation</h2>
          <p className="text-xs text-stone-500">Talk to your human</p>
        </div>
        <div className="flex-1 flex items-center justify-center text-xs text-stone-400">
          thread panel (placeholder)
        </div>
      </aside>
    </div>
  );
};

export default HumanPage;
