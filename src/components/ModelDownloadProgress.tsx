import { useEffect, useState } from 'react';

import { useModelStatus } from '../hooks/useModelStatus';

interface ModelDownloadProgressProps {
  className?: string;
  showWhenLoaded?: boolean;
}

const ModelDownloadProgress = ({
  className = '',
  showWhenLoaded = false,
}: ModelDownloadProgressProps) => {
  const { isAvailable, isLoaded, isLoading, downloadProgress, error, ensureLoaded } =
    useModelStatus();
  const [isMobile, setIsMobile] = useState(false);

  useEffect(() => {
    // Detect mobile platform
    const detectMobile = async () => {
      try {
        const { platform } = await import('@tauri-apps/plugin-os');
        const currentPlatform = await platform();
        setIsMobile(currentPlatform === 'android' || currentPlatform === 'ios');
      } catch {
        // If we can't detect platform, assume desktop
        setIsMobile(false);
      }
    };
    detectMobile();
  }, []);

  // Show mobile-only message on mobile platforms
  if (isMobile) {
    return (
      <div className={`glass rounded-2xl p-3 shadow-large animate-fade-up ${className}`}>
        <div className="flex items-center gap-3">
          <div className="flex-shrink-0">
            <svg
              className="w-5 h-5 text-stone-400"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M9.75 17L9 20l-1 1h8l-1-1-.75-3M3 13h18M5 17h14a2 2 0 002-2V5a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"
              />
            </svg>
          </div>
          <div className="flex-1 min-w-0">
            <span className="font-medium text-sm">Local AI Model</span>
            <p className="text-xs opacity-60">Available on desktop only</p>
          </div>
        </div>
      </div>
    );
  }

  // Don't render if not available on this platform
  if (!isAvailable) {
    return null;
  }

  // Don't render if loaded and showWhenLoaded is false
  if (isLoaded && !showWhenLoaded && !isLoading) {
    return null;
  }

  // Format download progress percentage
  const progressPercent = downloadProgress !== null ? Math.round(downloadProgress * 100) : 0;

  // Determine status display
  const getStatusDisplay = () => {
    if (error) {
      return {
        icon: (
          <svg
            className="w-5 h-5 text-coral-500"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"
            />
          </svg>
        ),
        title: 'Model Error',
        description: error,
        color: 'coral',
      };
    }

    if (isLoading && downloadProgress !== null) {
      return {
        icon: (
          <svg
            className="w-5 h-5 text-primary-500 animate-pulse"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4"
            />
          </svg>
        ),
        title: 'Downloading Local AI Model',
        description: `${progressPercent}% complete (~1.2 GB).`,
        color: 'primary',
      };
    }

    if (isLoading) {
      return {
        icon: (
          <svg
            className="w-5 h-5 text-primary-500 animate-spin"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"
            />
          </svg>
        ),
        title: 'Loading Local AI Model',
        description: 'Initializing local inference engine...',
        color: 'primary',
      };
    }

    if (isLoaded) {
      return {
        icon: (
          <svg
            className="w-5 h-5 text-sage-500"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
          </svg>
        ),
        title: 'AI Model Ready',
        description: 'Local inference available',
        color: 'sage',
      };
    }

    // Not loaded, show download prompt
    return {
      icon: (
        <svg
          className="w-5 h-5 text-stone-400"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M9.75 17L9 20l-1 1h8l-1-1-.75-3M3 13h18M5 17h14a2 2 0 002-2V5a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"
          />
        </svg>
      ),
      title: 'Local AI Model',
      description: 'Download for offline summarization',
      color: 'stone',
    };
  };

  const statusDisplay = getStatusDisplay();

  return (
    <div className={`glass rounded-2xl p-3 shadow-large animate-fade-up ${className}`}>
      <div className="flex items-center gap-3">
        {/* Icon */}
        <div className="flex-shrink-0">{statusDisplay.icon}</div>

        {/* Content */}
        <div className="flex-1 min-w-0">
          <div className="flex items-center justify-between mb-1">
            <span className="font-medium text-sm">{statusDisplay.title}</span>
            {!isLoaded && !isLoading && !error && (
              <button
                onClick={ensureLoaded}
                className="text-xs text-primary-500 hover:text-primary-400 transition-colors">
                Download
              </button>
            )}
            {error && (
              <button
                onClick={ensureLoaded}
                className="text-xs text-coral-500 hover:text-coral-400 transition-colors">
                Retry
              </button>
            )}
          </div>
          <p className="text-xs opacity-60 truncate">{statusDisplay.description}</p>

          {/* Progress bar */}
          {isLoading && downloadProgress !== null && (
            <div className="mt-2 w-full bg-stone-700/50 rounded-full h-1.5 overflow-hidden">
              <div
                className="bg-primary-500 h-full rounded-full transition-all duration-300 ease-out"
                style={{ width: `${progressPercent}%` }}
              />
            </div>
          )}
        </div>
      </div>
    </div>
  );
};

export default ModelDownloadProgress;
