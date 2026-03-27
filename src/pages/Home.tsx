import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import ConnectionIndicator from '../components/ConnectionIndicator';
import { useUser } from '../hooks/useUser';
import { isTauri, type LocalAiStatus, openhumanLocalAiStatus } from '../utils/tauriCommands';

const Home = () => {
  const { user } = useUser();
  const navigate = useNavigate();
  const userName = user?.firstName || 'User';
  const [localAiStatus, setLocalAiStatus] = useState<LocalAiStatus | null>(null);

  // Get greeting based on time
  const getGreeting = () => {
    const hour = new Date().getHours();
    if (hour < 12) return 'Good morning';
    if (hour < 18) return 'Good afternoon';
    return 'Good evening';
  };

  // Open in-app conversations window
  const handleStartCooking = async () => {
    navigate('/conversations');
  };

  useEffect(() => {
    if (!isTauri()) return;
    let mounted = true;
    const load = async () => {
      try {
        const status = await openhumanLocalAiStatus();
        if (mounted) setLocalAiStatus(status.result);
      } catch {
        if (mounted) setLocalAiStatus(null);
      }
    };
    void load();
    const timer = setInterval(() => void load(), 7000);
    return () => {
      mounted = false;
      clearInterval(timer);
    };
  }, []);

  return (
    <div className="min-h-full relative">
      {/* Content overlay */}
      <div className="relative z-10 min-h-full flex flex-col">
        {/* Main content */}
        <div className="flex-1 flex items-center justify-center p-4">
          <div className="max-w-md w-full">
            {/* Weather card */}
            <div className="glass rounded-3xl p-4 shadow-large animate-fade-up text-center">
              {/* Greeting */}
              <h1 className="text-2xl font-bold mb-4">
                {getGreeting()}, {userName}
              </h1>

              {/* Connection indicators */}
              <ConnectionIndicator />

              {localAiStatus?.warning && (
                <div className="my-3 rounded-xl border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-left text-xs text-amber-200">
                  Local model warning: {localAiStatus.warning}
                </div>
              )}

              {/* Get Access button */}
              <button
                onClick={handleStartCooking}
                className="btn-primary w-full py-2.5 text-sm font-medium rounded-xl">
                Message OpenHuman 🔥
              </button>
            </div>

            <div className="mt-4 mb-8">
              <button
                onClick={() => navigate('/skills')}
                className="btn-secondary w-full py-2.5 text-sm font-medium rounded-xl">
                Open Skills Page
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};

export default Home;
