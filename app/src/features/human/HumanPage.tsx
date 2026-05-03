import Conversations from '../../pages/Conversations';
import { Ghosty } from './Mascot';
import { useHumanMascot } from './useHumanMascot';

const HumanPage = () => {
  const { face, viseme } = useHumanMascot();
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
          <Ghosty face={face} viseme={viseme} />
        </div>
      </div>

      <aside className="w-[440px] border-l border-stone-300 bg-white flex flex-col overflow-hidden">
        <Conversations variant="sidebar" />
      </aside>
    </div>
  );
};

export default HumanPage;
