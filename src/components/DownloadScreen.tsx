import { useEffect, useState } from 'react';

import {
  detectPlatform,
  getDownloadLink,
  getPlatformDisplayName,
  type Platform,
  type PlatformInfo,
} from '../utils/deviceDetection';
import { isTauri } from '../utils/tauriCommands';

interface DownloadOption {
  platform: Platform;
  label: string;
  icon: string;
}

const downloadOptions: DownloadOption[] = [
  { platform: 'windows', label: 'Windows', icon: '🪟' },
  { platform: 'macos', label: 'macOS', icon: '🍎' },
  { platform: 'linux', label: 'Linux', icon: '🐧' },
];

const DownloadScreen = () => {
  const [platformInfo, setPlatformInfo] = useState<PlatformInfo | null>(null);
  const [selectedPlatform, setSelectedPlatform] = useState<Platform | null>(null);

  useEffect(() => {
    // Only show download screen on web (not in Tauri app)
    if (isTauri()) {
      return;
    }

    const detected = detectPlatform();
    setPlatformInfo(detected);
    setSelectedPlatform(detected.platform);
  }, []);

  // Don't render if running in Tauri or platform not detected
  if (isTauri() || !platformInfo || !selectedPlatform) {
    return null;
  }

  const downloadUrl = getDownloadLink(selectedPlatform);
  const platformName = getPlatformDisplayName(selectedPlatform);

  const handleDownload = () => {
    window.open(downloadUrl, '_blank');
  };

  return (
    <div className="animate-fade-up">
      {/* Auto-detected platform */}
      <div className="mb-6">
        <div className="bg-blue-500/10 border border-blue-500/20 rounded-xl p-4 mb-4">
          <div className="flex items-center justify-between">
            <div className="flex items-center space-x-3">
              <span className="text-2xl">
                {downloadOptions.find(opt => opt.platform === selectedPlatform)?.icon}
              </span>
              <div>
                <p className="text-sm opacity-70">Recommended for you</p>
                <p className="font-semibold">{platformName}</p>
              </div>
            </div>
            <button
              onClick={handleDownload}
              className="bg-blue-500 hover:bg-blue-600 active:bg-blue-700 text-white font-semibold px-6 py-2 rounded-lg transition-all duration-300 hover:shadow-medium hover:scale-[1.02] active:scale-[0.98]">
              Download
            </button>
          </div>
        </div>
      </div>

      {/* Other platforms */}
      <div className="border-t border-white/10 pt-6">
        <p className="text-sm opacity-70 mb-4 text-center">Or download for other platforms:</p>
        <div className="grid grid-cols-2 gap-3">
          {downloadOptions
            .filter(opt => opt.platform !== selectedPlatform)
            .map(option => (
              <button
                key={option.platform}
                onClick={() => {
                  setSelectedPlatform(option.platform);
                  window.open(getDownloadLink(option.platform), '_blank');
                }}
                className="flex items-center justify-center space-x-2 bg-white/5 hover:bg-white/10 border border-white/10 hover:border-white/20 rounded-lg p-3 transition-all duration-300 hover:scale-[1.02] active:scale-[0.98]">
                <span className="text-xl">{option.icon}</span>
                <span className="text-sm font-medium">{option.label}</span>
              </button>
            ))}
        </div>
      </div>
    </div>
  );
};

export default DownloadScreen;
