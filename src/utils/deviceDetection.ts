/**
 * Device Detection Utility
 *
 * Detects the user's platform/device type for showing appropriate download links
 */

export type Platform = 'windows' | 'macos' | 'linux' | 'android' | 'ios' | 'unknown';

export interface PlatformInfo {
  platform: Platform;
  isMobile: boolean;
  isDesktop: boolean;
}

/**
 * Detect the user's platform based on user agent
 */
export function detectPlatform(): PlatformInfo {
  if (typeof window === 'undefined') {
    return { platform: 'unknown', isMobile: false, isDesktop: false };
  }

  const userAgent = window.navigator.userAgent.toLowerCase();
  const platform = window.navigator.platform.toLowerCase();

  // Mobile detection
  const isMobile = /android|webos|iphone|ipad|ipod|blackberry|iemobile|opera mini/i.test(userAgent);

  // iOS detection
  if (/iphone|ipad|ipod/i.test(userAgent)) {
    return { platform: 'ios', isMobile: true, isDesktop: false };
  }

  // Android detection
  if (/android/i.test(userAgent)) {
    return { platform: 'android', isMobile: true, isDesktop: false };
  }

  // Windows detection
  if (/win/i.test(platform) || /windows/i.test(userAgent)) {
    return { platform: 'windows', isMobile: false, isDesktop: true };
  }

  // macOS detection
  if (/mac/i.test(platform) || /macintosh/i.test(userAgent)) {
    return { platform: 'macos', isMobile: false, isDesktop: true };
  }

  // Linux detection
  if (/linux/i.test(platform) && !/android/i.test(userAgent)) {
    return { platform: 'linux', isMobile: false, isDesktop: true };
  }

  return { platform: 'unknown', isMobile, isDesktop: !isMobile };
}

/**
 * Get download link for a specific platform
 */
export function getDownloadLink(platform: Platform): string {
  // Dummy links for now - replace with actual download URLs later
  const links: Record<Platform, string> = {
    windows: 'https://example.com/download/alphahuman-windows.exe',
    macos: 'https://example.com/download/alphahuman-macos.dmg',
    linux: 'https://example.com/download/alphahuman-linux.AppImage',
    android: 'https://example.com/download/alphahuman-android.apk',
    ios: 'https://apps.apple.com/app/alphahuman',
    unknown: 'https://example.com/download',
  };

  return links[platform];
}

/**
 * Get platform display name
 */
export function getPlatformDisplayName(platform: Platform): string {
  const names: Record<Platform, string> = {
    windows: 'Windows',
    macos: 'macOS',
    linux: 'Linux',
    android: 'Android',
    ios: 'iOS',
    unknown: 'Your Device',
  };

  return names[platform];
}
