import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const INFO_PLIST_PATH = path.resolve(HERE, '..', 'src-tauri', 'Info.plist');

const REQUIRED_PRIVACY_KEYS = [
  'NSMicrophoneUsageDescription',
  'NSCameraUsageDescription',
  'NSAppleEventsUsageDescription',
  'NSBluetoothAlwaysUsageDescription',
  'NSLocationWhenInUseUsageDescription',
  'NSDocumentsFolderUsageDescription',
  'NSDownloadsFolderUsageDescription',
  'NSDesktopFolderUsageDescription',
  'NSContactsUsageDescription',
  'NSCalendarsUsageDescription',
] as const;

const MIN_DESCRIPTION_LEN = 30;

function parsePlistKeyValuePairs(xml: string): Map<string, string> {
  const doc = new DOMParser().parseFromString(xml, 'application/xml');
  const parserError = doc.querySelector('parsererror');
  if (parserError) {
    throw new Error(`Info.plist XML parse failed: ${parserError.textContent ?? 'unknown'}`);
  }

  const root = doc.querySelector('plist > dict');
  if (!root) {
    throw new Error('Info.plist missing top-level <plist><dict>');
  }

  const map = new Map<string, string>();
  const children = Array.from(root.children);
  for (let i = 0; i < children.length; i++) {
    const node = children[i];
    if (node.tagName !== 'key') continue;
    const keyName = (node.textContent ?? '').trim();
    const valueNode = children[i + 1];
    if (!valueNode || valueNode.tagName !== 'string') continue;
    map.set(keyName, (valueNode.textContent ?? '').trim());
  }
  return map;
}

describe('app/src-tauri/Info.plist macOS privacy keys', () => {
  const xml = readFileSync(INFO_PLIST_PATH, 'utf8');
  const pairs = parsePlistKeyValuePairs(xml);

  it.each(REQUIRED_PRIVACY_KEYS)('declares %s with non-empty user-facing copy', key => {
    const value = pairs.get(key);
    expect(value, `Missing required privacy key '${key}' in Info.plist`).toBeDefined();
    expect(
      (value ?? '').length,
      `Privacy key '${key}' description must be ≥ ${MIN_DESCRIPTION_LEN} chars (placeholder check)`
    ).toBeGreaterThanOrEqual(MIN_DESCRIPTION_LEN);
  });

  it('has the same key count as REQUIRED_PRIVACY_KEYS or strictly more', () => {
    const declared = REQUIRED_PRIVACY_KEYS.filter(k => pairs.has(k)).length;
    expect(declared).toBe(REQUIRED_PRIVACY_KEYS.length);
  });
});
