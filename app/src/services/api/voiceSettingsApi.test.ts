import { describe, expect, it } from 'vitest';

import {
  parseVoiceProviderString,
  serializeVoiceProviderRef,
  type VoiceProviderRef,
} from './voiceSettingsApi';

describe('parseVoiceProviderString', () => {
  it('parses null/undefined/empty to cloud', () => {
    expect(parseVoiceProviderString(null)).toEqual({ kind: 'cloud' });
    expect(parseVoiceProviderString(undefined)).toEqual({ kind: 'cloud' });
    expect(parseVoiceProviderString('')).toEqual({ kind: 'cloud' });
    expect(parseVoiceProviderString('  ')).toEqual({ kind: 'cloud' });
  });

  it('parses "cloud" sentinel', () => {
    expect(parseVoiceProviderString('cloud')).toEqual({ kind: 'cloud' });
  });

  it('parses "openhuman" sentinel', () => {
    expect(parseVoiceProviderString('openhuman')).toEqual({ kind: 'cloud' });
  });

  // `"whisper"` selected the removed local engine. It is no longer a local
  // sentinel, so it parses as an external slug and the factory errors on it by
  // name rather than silently resolving to something else.
  it('parses the removed "whisper" sentinel as a plain unknown string', () => {
    expect(parseVoiceProviderString('whisper')).toEqual({ kind: 'cloud' });
  });

  it('parses "piper" to local', () => {
    expect(parseVoiceProviderString('piper')).toEqual({
      kind: 'local',
      engine: 'piper',
      model: '',
    });
  });

  it('parses "whisper:large-v3-turbo" as an external slug, not a local engine', () => {
    expect(parseVoiceProviderString('whisper:large-v3-turbo')).toEqual({
      kind: 'external',
      providerSlug: 'whisper',
      model: 'large-v3-turbo',
    });
  });

  it('parses "piper:en_US-lessac-medium" to local with model', () => {
    expect(parseVoiceProviderString('piper:en_US-lessac-medium')).toEqual({
      kind: 'local',
      engine: 'piper',
      model: 'en_US-lessac-medium',
    });
  });

  it('parses "deepgram:nova-2" to external', () => {
    expect(parseVoiceProviderString('deepgram:nova-2')).toEqual({
      kind: 'external',
      providerSlug: 'deepgram',
      model: 'nova-2',
    });
  });

  it('parses "openai:whisper-1" to external', () => {
    expect(parseVoiceProviderString('openai:whisper-1')).toEqual({
      kind: 'external',
      providerSlug: 'openai',
      model: 'whisper-1',
    });
  });

  it('parses "elevenlabs:voice-id-123" to external', () => {
    expect(parseVoiceProviderString('elevenlabs:voice-id-123')).toEqual({
      kind: 'external',
      providerSlug: 'elevenlabs',
      model: 'voice-id-123',
    });
  });

  it('parses "openai:alloy" to external', () => {
    expect(parseVoiceProviderString('openai:alloy')).toEqual({
      kind: 'external',
      providerSlug: 'openai',
      model: 'alloy',
    });
  });

  it('parses "custom:my-model" to external', () => {
    expect(parseVoiceProviderString('custom:my-model')).toEqual({
      kind: 'external',
      providerSlug: 'custom',
      model: 'my-model',
    });
  });

  it('handles model with colons in it', () => {
    expect(parseVoiceProviderString('custom:model:v2')).toEqual({
      kind: 'external',
      providerSlug: 'custom',
      model: 'model:v2',
    });
  });

  it('falls back to cloud for unknown bare string', () => {
    expect(parseVoiceProviderString('unknown')).toEqual({ kind: 'cloud' });
  });

  it('trims whitespace', () => {
    expect(parseVoiceProviderString('  cloud  ')).toEqual({ kind: 'cloud' });
    expect(parseVoiceProviderString('  piper  ')).toEqual({
      kind: 'local',
      engine: 'piper',
      model: '',
    });
  });
});

describe('serializeVoiceProviderRef', () => {
  it('serializes cloud', () => {
    expect(serializeVoiceProviderRef({ kind: 'cloud' })).toBe('cloud');
  });

  it('serializes local piper without model', () => {
    expect(serializeVoiceProviderRef({ kind: 'local', engine: 'piper', model: '' })).toBe('piper');
  });

  it('serializes local piper with model', () => {
    expect(
      serializeVoiceProviderRef({ kind: 'local', engine: 'piper', model: 'en_US-lessac-medium' })
    ).toBe('piper:en_US-lessac-medium');
  });

  it('serializes external with model', () => {
    expect(
      serializeVoiceProviderRef({ kind: 'external', providerSlug: 'deepgram', model: 'nova-2' })
    ).toBe('deepgram:nova-2');
  });

  it('serializes external without model', () => {
    expect(
      serializeVoiceProviderRef({ kind: 'external', providerSlug: 'deepgram', model: '' })
    ).toBe('deepgram');
  });
});

describe('parseVoiceProviderString / serializeVoiceProviderRef round-trip', () => {
  const cases: [string, VoiceProviderRef][] = [
    ['cloud', { kind: 'cloud' }],
    ['piper', { kind: 'local', engine: 'piper', model: '' }],
    ['piper:en_US-lessac-medium', { kind: 'local', engine: 'piper', model: 'en_US-lessac-medium' }],
    ['deepgram:nova-2', { kind: 'external', providerSlug: 'deepgram', model: 'nova-2' }],
    ['openai:whisper-1', { kind: 'external', providerSlug: 'openai', model: 'whisper-1' }],
    ['openai:alloy', { kind: 'external', providerSlug: 'openai', model: 'alloy' }],
    ['elevenlabs:voice-id', { kind: 'external', providerSlug: 'elevenlabs', model: 'voice-id' }],
  ];

  for (const [wire, ref_] of cases) {
    it(`round-trips "${wire}"`, () => {
      const parsed = parseVoiceProviderString(wire);
      expect(parsed).toEqual(ref_);
      expect(serializeVoiceProviderRef(parsed)).toBe(wire);
    });
  }
});
