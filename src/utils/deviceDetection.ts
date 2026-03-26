/**
 * Device Detection Utility
 *
 * Detects the user's platform/device type for showing appropriate download links
 */

export type Platform = 'windows' | 'macos' | 'linux' | 'android' | 'ios' | 'unknown';

export type Architecture = 'x64' | 'aarch64' | 'amd64' | 'x86_64' | 'arm64' | 'unknown';

export interface PlatformInfo {
  platform: Platform;
  architecture: Architecture;
  isMobile: boolean;
  isDesktop: boolean;
}

export interface GitHubReleaseAsset {
  name: string;
  browser_download_url: string;
  content_type: string;
  size: number;
}

export interface GitHubRelease {
  tag_name: string;
  name: string;
  assets: GitHubReleaseAsset[];
}

export interface PlatformDownloadLinks {
  windows?: string;
  macos?: string;
  linux?: string;
  android?: string;
  ios?: string;
}

export interface ArchitectureDownloadLink {
  architecture: Architecture;
  displayName: string;
  url: string;
  fileName: string;
}

export interface PlatformArchitectureLinks {
  windows?: ArchitectureDownloadLink[];
  macos?: ArchitectureDownloadLink[];
  linux?: ArchitectureDownloadLink[];
  android?: ArchitectureDownloadLink[];
}

/**
 * Detect the user's architecture based on user agent and platform
 */
export function detectArchitecture(): Architecture {
  if (typeof window === 'undefined') {
    return 'unknown';
  }

  const userAgent = window.navigator.userAgent.toLowerCase();
  const platform = window.navigator.platform.toLowerCase();

  // Check for ARM architecture (Apple Silicon, ARM64)
  if (/arm64|aarch64/i.test(userAgent) || /arm64|aarch64/i.test(platform)) {
    return 'aarch64';
  }

  // Check for x64/amd64/x86_64
  if (/x64|amd64|x86_64|win64|wow64/i.test(userAgent) || /x64|amd64|x86_64/i.test(platform)) {
    return 'x64';
  }

  // macOS on Apple Silicon
  if (/mac/i.test(platform) && /cpu os \d+_\d+_0 like mac os x/i.test(userAgent)) {
    // Check if it's Apple Silicon (M1/M2/etc) - newer macOS versions
    const match = userAgent.match(/cpu os (\d+)_(\d+)_/);
    if (match) {
      const major = parseInt(match[1], 10);
      // macOS 11+ on Apple Silicon
      if (major >= 11) {
        // Try to detect ARM - if navigator.hardwareConcurrency suggests ARM, or check for Rosetta
        // For now, default to x64 for macOS unless explicitly ARM
        // We'll rely on the user agent check above
      }
    }
  }

  // Default to x64 for desktop platforms
  if (/win|mac|linux/i.test(platform)) {
    return 'x64';
  }

  return 'unknown';
}

/**
 * Detect the user's platform based on user agent
 */
export function detectPlatform(): PlatformInfo {
  if (typeof window === 'undefined') {
    return { platform: 'unknown', architecture: 'unknown', isMobile: false, isDesktop: false };
  }

  const userAgent = window.navigator.userAgent.toLowerCase();
  const platform = window.navigator.platform.toLowerCase();
  const architecture = detectArchitecture();

  // Mobile detection
  const isMobile = /android|webos|iphone|ipad|ipod|blackberry|iemobile|opera mini/i.test(userAgent);

  // iOS detection
  if (/iphone|ipad|ipod/i.test(userAgent)) {
    return { platform: 'ios', architecture: 'aarch64', isMobile: true, isDesktop: false };
  }

  // Android detection
  if (/android/i.test(userAgent)) {
    return { platform: 'android', architecture, isMobile: true, isDesktop: false };
  }

  // Windows detection
  if (/win/i.test(platform) || /windows/i.test(userAgent)) {
    return { platform: 'windows', architecture, isMobile: false, isDesktop: true };
  }

  // macOS detection
  if (/mac/i.test(platform) || /macintosh/i.test(userAgent)) {
    return { platform: 'macos', architecture, isMobile: false, isDesktop: true };
  }

  // Linux detection
  if (/linux/i.test(platform) && !/android/i.test(userAgent)) {
    return { platform: 'linux', architecture, isMobile: false, isDesktop: true };
  }

  return { platform: 'unknown', architecture, isMobile, isDesktop: !isMobile };
}

/**
 * Fetch the latest release from GitHub
 */
export async function fetchLatestRelease(): Promise<GitHubRelease> {
  const response = await fetch(
    'https://api.github.com/repos/openhumanxyz/openhuman/releases/latest'
  );
  if (!response.ok) {
    throw new Error(`Failed to fetch release: ${response.statusText}`);
  }
  return response.json();
}

/**
 * Extract architecture from asset filename
 */
function extractArchitecture(name: string): Architecture {
  const lowerName = name.toLowerCase();

  if (lowerName.includes('aarch64') || lowerName.includes('arm64')) {
    return 'aarch64';
  }
  if (lowerName.includes('amd64') || lowerName.includes('x86_64') || lowerName.includes('x64')) {
    return 'x64';
  }
  if (lowerName.includes('x86') || lowerName.includes('i386')) {
    return 'x64'; // Treat x86 as x64 for compatibility
  }

  return 'unknown';
}

/**
 * Get architecture display name
 */
export function getArchitectureDisplayName(arch: Architecture): string {
  const names: Record<Architecture, string> = {
    x64: 'Intel (x64)',
    aarch64: 'Apple Silicon (ARM64)',
    amd64: 'AMD64',
    x86_64: 'x86_64',
    arm64: 'ARM64',
    unknown: 'Unknown',
  };
  return names[arch] || arch;
}

/**
 * Parse GitHub release assets and map them to platforms with architecture support
 */
export function parseReleaseAssetsByArchitecture(
  assets: GitHubReleaseAsset[]
): PlatformArchitectureLinks {
  const links: PlatformArchitectureLinks = {};

  // Use Maps to track unique architectures per platform
  const windowsMap = new Map<Architecture, ArchitectureDownloadLink>();
  const macosMap = new Map<Architecture, ArchitectureDownloadLink>();
  const linuxMap = new Map<Architecture, ArchitectureDownloadLink>();
  const androidMap = new Map<Architecture, ArchitectureDownloadLink>();

  for (const asset of assets) {
    const name = asset.name.toLowerCase();

    // Skip signature files
    if (name.endsWith('.sig')) {
      continue;
    }

    const architecture = extractArchitecture(asset.name);
    const displayName = getArchitectureDisplayName(architecture);
    const link: ArchitectureDownloadLink = {
      architecture,
      displayName,
      url: asset.browser_download_url,
      fileName: asset.name,
    };

    // Windows: .exe, .msi, .zip (Windows)
    if (
      name.includes('windows') ||
      name.includes('.exe') ||
      name.includes('.msi') ||
      (name.includes('.zip') && !name.includes('macos'))
    ) {
      // Only add if this architecture doesn't exist yet, or prefer more specific filenames
      if (
        !windowsMap.has(architecture) ||
        (name.includes('windows') &&
          !windowsMap.get(architecture)?.fileName.toLowerCase().includes('windows'))
      ) {
        windowsMap.set(architecture, link);
      }
    }
    // macOS: .dmg
    else if (name.includes('macos') || name.includes('.dmg') || name.includes('darwin')) {
      // Only add if this architecture doesn't exist yet, or prefer more specific filenames
      if (
        !macosMap.has(architecture) ||
        (name.includes('macos') &&
          !macosMap.get(architecture)?.fileName.toLowerCase().includes('macos'))
      ) {
        macosMap.set(architecture, link);
      }
    }
    // Linux: .AppImage, .deb, .rpm
    else if (
      name.includes('linux') ||
      name.includes('.appimage') ||
      name.includes('.deb') ||
      name.includes('.rpm')
    ) {
      // Prefer AppImage, then deb, then rpm for the same architecture
      const existing = linuxMap.get(architecture);
      if (!existing) {
        linuxMap.set(architecture, link);
      } else {
        const existingName = existing.fileName.toLowerCase();
        // Prefer AppImage over others
        if (name.includes('.appimage') && !existingName.includes('.appimage')) {
          linuxMap.set(architecture, link);
        }
        // Prefer deb over rpm
        else if (name.includes('.deb') && existingName.includes('.rpm')) {
          linuxMap.set(architecture, link);
        }
      }
    }
    // Android: .apk
    else if (name.includes('android') || name.includes('.apk')) {
      // Only add if this architecture doesn't exist yet
      if (!androidMap.has(architecture)) {
        androidMap.set(architecture, link);
      }
    }
  }

  // Convert Maps to arrays
  if (windowsMap.size > 0) {
    links.windows = Array.from(windowsMap.values());
  }
  if (macosMap.size > 0) {
    links.macos = Array.from(macosMap.values());
  }
  if (linuxMap.size > 0) {
    links.linux = Array.from(linuxMap.values());
  }
  if (androidMap.size > 0) {
    links.android = Array.from(androidMap.values());
  }

  // Sort architectures: prefer detected architecture, then x64, then aarch64
  const sortArchitectures = (
    archLinks: ArchitectureDownloadLink[],
    preferredArch?: Architecture
  ) => {
    return archLinks.sort((a, b) => {
      if (preferredArch && a.architecture === preferredArch) return -1;
      if (preferredArch && b.architecture === preferredArch) return 1;
      if (a.architecture === 'x64') return -1;
      if (b.architecture === 'x64') return 1;
      if (a.architecture === 'aarch64') return -1;
      if (b.architecture === 'aarch64') return 1;
      return 0;
    });
  };

  if (links.windows) {
    links.windows = sortArchitectures(links.windows);
  }
  if (links.macos) {
    links.macos = sortArchitectures(links.macos);
  }
  if (links.linux) {
    links.linux = sortArchitectures(links.linux);
  }
  if (links.android) {
    links.android = sortArchitectures(links.android);
  }

  return links;
}

/**
 * Parse GitHub release assets and map them to platforms (legacy - for backward compatibility)
 */
export function parseReleaseAssets(assets: GitHubReleaseAsset[]): PlatformDownloadLinks {
  const archLinks = parseReleaseAssetsByArchitecture(assets);
  const links: PlatformDownloadLinks = {};

  // Get the first (preferred) link for each platform
  if (archLinks.windows && archLinks.windows.length > 0) {
    links.windows = archLinks.windows[0].url;
  }
  if (archLinks.macos && archLinks.macos.length > 0) {
    links.macos = archLinks.macos[0].url;
  }
  if (archLinks.linux && archLinks.linux.length > 0) {
    links.linux = archLinks.linux[0].url;
  }
  if (archLinks.android && archLinks.android.length > 0) {
    links.android = archLinks.android[0].url;
  }

  return links;
}

/**
 * Get download link for a specific platform (fallback to dummy links)
 */
export function getDownloadLink(platform: Platform, releaseLinks?: PlatformDownloadLinks): string {
  // Use release links if available (skip 'unknown' and 'ios' as they're not in PlatformDownloadLinks)
  if (releaseLinks && platform !== 'unknown' && platform !== 'ios') {
    if (platform === 'windows' && releaseLinks.windows) {
      return releaseLinks.windows;
    }
    if (platform === 'macos' && releaseLinks.macos) {
      return releaseLinks.macos;
    }
    if (platform === 'linux' && releaseLinks.linux) {
      return releaseLinks.linux;
    }
    if (platform === 'android' && releaseLinks.android) {
      return releaseLinks.android;
    }
  }

  // Fallback to dummy links
  const links: Record<Platform, string> = {
    windows: 'https://tryopenhuman.com/download/openhuman-windows.exe',
    macos: 'https://tryopenhuman.com/download/openhuman-macos.dmg',
    linux: 'https://tryopenhuman.com/download/openhuman-linux.AppImage',
    android: 'https://tryopenhuman.com/download/openhuman-android.apk',
    ios: 'https://apps.apple.com/app/openhuman',
    unknown: 'https://tryopenhuman.com/download',
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
