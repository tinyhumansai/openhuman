import { useCallback } from 'react';

import DownloadScreen from '../components/DownloadScreen';
import TelegramLoginButton from '../components/TelegramLoginButton';
import TypewriterGreeting from '../components/TypewriterGreeting';
import { useModelStatus } from '../hooks/useModelStatus';

interface WelcomeProps {
  isWeb: boolean;
}

const Welcome = ({ isWeb }: WelcomeProps) => {
  const greetings = [
    'Hello HAL9000! 👋',
    "Let's cook! 🔥",
    'The A-Team is here! 👊',
  ];

  const { isAvailable, isDownloaded, isLoading, downloadProgress, error, startDownload } =
    useModelStatus();

  const handleRetry = useCallback(() => {
    startDownload();
  }, [startDownload]);

  const progressPercent = downloadProgress !== null ? Math.round(downloadProgress * 100) : 0;

  // Determine what to show for download progress
  const showProgress = !isWeb && isAvailable && !isDownloaded;
  const isDownloading = isLoading && downloadProgress !== null;
  const isPreparing = isLoading && downloadProgress === null;

  return (
    <div className="min-h-full relative flex items-center justify-center">
      {/* Main content */}
      <div className="relative z-10 max-w-md w-full mx-4 space-y-6">
        {/* Welcome card */}
        <div className="glass rounded-3xl p-8 text-center animate-fade-up shadow-large">
          {/* Greeting */}
          <TypewriterGreeting greetings={greetings} />

          <p className="opacity-70 mb-8 leading-relaxed">
            Welcome to AlphaHuman. Your Telegram assistant here to get you 10x more done in your
            journey.
          </p>

          <p className="opacity-70 leading-relaxed">Are you ready for this?</p>

          {/* Model download progress (desktop only) */}
          {showProgress && (
            <div className="mt-6">
              {isDownloading && (
                <div className="space-y-2">
                  <div className="w-full bg-stone-700/30 rounded-full h-1.5 overflow-hidden">
                    <div
                      className="bg-primary-500/80 h-full rounded-full transition-all duration-300 ease-out"
                      style={{ width: `${progressPercent}%` }}
                    />
                  </div>
                  <p className="text-xs opacity-50">
                    Downloading AI model... {progressPercent}%
                    <span className="ml-1 opacity-70">(~1.2 GB)</span>
                  </p>
                </div>
              )}

              {isPreparing && (
                <p className="text-xs opacity-50">Preparing AI model download...</p>
              )}

              {error && !isLoading && (
                <div className="space-y-2">
                  <p className="text-xs text-coral-500/80">{error}</p>
                  <button
                    onClick={handleRetry}
                    className="text-xs text-primary-500 hover:text-primary-400 transition-colors">
                    Retry download
                  </button>
                </div>
              )}
            </div>
          )}

          {/* Show Telegram login button in Tauri app, download screen on web */}
          {!isWeb && (
            <div className="mt-6">
              <TelegramLoginButton />
            </div>
          )}
        </div>

        {isWeb && <DownloadScreen />}

        {/* Bottom text */}
        <p className="text-center opacity-60 text-sm">Made with ❤️ by a bunch of Web3 nerds</p>
      </div>
    </div>
  );
};

export default Welcome;
