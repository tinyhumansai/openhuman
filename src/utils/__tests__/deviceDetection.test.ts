import { describe, expect, it } from 'vitest';

import {
  getArchitectureDisplayName,
  getDownloadLink,
  getPlatformDisplayName,
  type GitHubReleaseAsset,
  parseReleaseAssetsByArchitecture,
} from '../deviceDetection';

describe('getArchitectureDisplayName', () => {
  it('returns correct names for known architectures', () => {
    expect(getArchitectureDisplayName('x64')).toBe('Intel (x64)');
    expect(getArchitectureDisplayName('aarch64')).toBe('Apple Silicon (ARM64)');
    expect(getArchitectureDisplayName('amd64')).toBe('AMD64');
    expect(getArchitectureDisplayName('unknown')).toBe('Unknown');
  });
});

describe('getPlatformDisplayName', () => {
  it('returns human-readable platform names', () => {
    expect(getPlatformDisplayName('windows')).toBe('Windows');
    expect(getPlatformDisplayName('macos')).toBe('macOS');
    expect(getPlatformDisplayName('linux')).toBe('Linux');
    expect(getPlatformDisplayName('android')).toBe('Android');
    expect(getPlatformDisplayName('ios')).toBe('iOS');
    expect(getPlatformDisplayName('unknown')).toBe('Your Device');
  });
});

describe('getDownloadLink', () => {
  it('returns release links when available', () => {
    const releaseLinks = {
      windows: 'https://releases.example.com/app.exe',
      macos: 'https://releases.example.com/app.dmg',
    };
    expect(getDownloadLink('windows', releaseLinks)).toBe('https://releases.example.com/app.exe');
    expect(getDownloadLink('macos', releaseLinks)).toBe('https://releases.example.com/app.dmg');
  });

  it('falls back to dummy links when no release links', () => {
    const link = getDownloadLink('linux');
    expect(link).toContain('example.com');
    expect(link).toContain('linux');
  });

  it('falls back for unknown and ios platforms', () => {
    const releaseLinks = { windows: 'https://example.com/w.exe' };
    const unknownLink = getDownloadLink('unknown', releaseLinks);
    expect(unknownLink).toBe('https://example.com/download');

    const iosLink = getDownloadLink('ios', releaseLinks);
    expect(iosLink).toContain('apple.com');
  });
});

describe('parseReleaseAssetsByArchitecture', () => {
  const makeAsset = (name: string, url?: string): GitHubReleaseAsset => ({
    name,
    browser_download_url: url || `https://example.com/${name}`,
    content_type: 'application/octet-stream',
    size: 1000,
  });

  it('categorizes assets by platform', () => {
    const assets = [
      makeAsset('openhuman-windows-x64-setup.exe'),
      makeAsset('openhuman-macos-aarch64.dmg'),
      makeAsset('openhuman-linux-x64.AppImage'),
      makeAsset('openhuman-android-arm64.apk'),
    ];

    const links = parseReleaseAssetsByArchitecture(assets);
    expect(links.windows).toHaveLength(1);
    expect(links.macos).toHaveLength(1);
    expect(links.linux).toHaveLength(1);
    expect(links.android).toHaveLength(1);
  });

  it('skips .sig signature files', () => {
    const assets = [
      makeAsset('openhuman-windows-x64-setup.exe'),
      makeAsset('openhuman-windows-x64-setup.exe.sig'),
    ];

    const links = parseReleaseAssetsByArchitecture(assets);
    expect(links.windows).toHaveLength(1);
  });

  it('groups multiple architectures per platform', () => {
    const assets = [makeAsset('openhuman-macos-x64.dmg'), makeAsset('openhuman-macos-aarch64.dmg')];

    const links = parseReleaseAssetsByArchitecture(assets);
    expect(links.macos).toHaveLength(2);
  });

  it('returns empty when no matching assets', () => {
    const links = parseReleaseAssetsByArchitecture([]);
    expect(links.windows).toBeUndefined();
    expect(links.macos).toBeUndefined();
    expect(links.linux).toBeUndefined();
    expect(links.android).toBeUndefined();
  });

  it('prefers AppImage over deb/rpm for Linux', () => {
    const assets = [
      makeAsset('openhuman-linux-x64.rpm'),
      makeAsset('openhuman-linux-x64.AppImage'),
    ];

    const links = parseReleaseAssetsByArchitecture(assets);
    expect(links.linux).toHaveLength(1);
    expect(links.linux![0].fileName).toContain('.AppImage');
  });
});
