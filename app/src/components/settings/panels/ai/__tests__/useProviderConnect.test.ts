import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { type AISettings, EMPTY_SETTINGS } from '../aiPanelTypes';
import { useProviderConnect } from '../useProviderConnect';

const api = vi.hoisted(() => ({
  flushCloudProviders: vi.fn(),
  listProviderModels: vi.fn(),
  loadProviderAuthErrors: vi.fn(),
  setCloudProviderKey: vi.fn(),
}));

vi.mock('../../../../../services/api/aiSettingsApi', async importOriginal => ({
  ...(await importOriginal<typeof import('../../../../../services/api/aiSettingsApi')>()),
  flushCloudProviders: api.flushCloudProviders,
  listProviderModels: api.listProviderModels,
  loadProviderAuthErrors: api.loadProviderAuthErrors,
  setCloudProviderKey: api.setCloudProviderKey,
}));

beforeEach(() => {
  api.flushCloudProviders.mockReset();
  api.flushCloudProviders.mockResolvedValue(undefined);
  api.listProviderModels.mockReset();
  api.listProviderModels.mockResolvedValue([]);
  api.loadProviderAuthErrors.mockReset();
  api.loadProviderAuthErrors.mockResolvedValue([]);
  api.setCloudProviderKey.mockReset();
  api.setCloudProviderKey.mockResolvedValue(undefined);
});

describe('useProviderConnect', () => {
  it('preserves an existing provider CA when saving a replacement connection', async () => {
    const existing = {
      id: 'provider-1',
      slug: 'openai',
      label: 'OpenAI',
      endpoint: 'https://api.openai.com/v1',
      authStyle: 'bearer' as const,
      maskedKey: '••••old',
      caCertPem: 'saved CA PEM',
    };
    const saved: AISettings = { ...EMPTY_SETTINGS, cloudProviders: [existing] };
    const persist = vi.fn().mockResolvedValue(undefined);
    const { result } = renderHook(() =>
      useProviderConnect({ draft: saved, saved, persist, t: key => key, onConnected: vi.fn() })
    );

    await act(async () => {
      await result.current.connectProvider({
        slug: 'openai',
        value: 'replacement-key',
        credentialMode: 'api_key',
      });
    });

    expect(api.flushCloudProviders).toHaveBeenCalledWith(
      expect.arrayContaining([
        expect.objectContaining({ slug: 'openai', ca_cert_pem: 'saved CA PEM' }),
      ])
    );
    expect(persist).toHaveBeenCalledWith(
      expect.objectContaining({
        cloudProviders: expect.arrayContaining([
          expect.objectContaining({ caCertPem: 'saved CA PEM' }),
        ]),
      })
    );
  });

  it('preserves a CA when reconnecting a provider with a legacy slug format', async () => {
    const existing = {
      id: 'provider-legacy',
      slug: ' OpenAI ',
      label: 'OpenAI',
      endpoint: 'https://api.openai.com/v1',
      authStyle: 'bearer' as const,
      maskedKey: '••••old',
      caCertPem: 'saved CA PEM',
    };
    const settings: AISettings = { ...EMPTY_SETTINGS, cloudProviders: [existing] };
    const { result } = renderHook(() =>
      useProviderConnect({
        draft: settings,
        saved: settings,
        persist: vi.fn().mockResolvedValue(undefined),
        t: key => key,
        onConnected: vi.fn(),
      })
    );

    await act(async () => {
      await result.current.connectProvider({
        slug: 'openai',
        value: 'replacement-key',
        credentialMode: 'api_key',
      });
    });

    expect(api.flushCloudProviders).toHaveBeenCalledWith([
      expect.objectContaining({ slug: 'openai', ca_cert_pem: 'saved CA PEM' }),
    ]);
  });

  it('uses the latest provider CA when a connection update is in flight', async () => {
    let finishKeyWrite!: () => void;
    api.setCloudProviderKey.mockImplementation(
      () => new Promise<void>(resolve => (finishKeyWrite = resolve))
    );
    const provider = {
      id: 'provider-1',
      slug: 'openai',
      label: 'OpenAI',
      endpoint: 'https://api.openai.com/v1',
      authStyle: 'bearer' as const,
      maskedKey: '••••old',
      caCertPem: 'old CA PEM',
    };
    const saved: AISettings = { ...EMPTY_SETTINGS, cloudProviders: [provider] };
    const persist = vi.fn().mockResolvedValue(undefined);
    const { result, rerender } = renderHook(
      ({ settings }) =>
        useProviderConnect({
          draft: settings,
          saved: settings,
          persist,
          t: key => key,
          onConnected: vi.fn(),
        }),
      { initialProps: { settings: saved } }
    );

    let connect!: Promise<void>;
    act(() => {
      connect = result.current.connectProvider({
        slug: 'openai',
        value: 'replacement-key',
        credentialMode: 'api_key',
      });
    });
    await waitFor(() => expect(finishKeyWrite).toBeTypeOf('function'));

    const latest: AISettings = {
      ...saved,
      cloudProviders: [{ ...provider, caCertPem: 'new CA PEM' }],
    };
    rerender({ settings: latest });
    await act(async () => {
      finishKeyWrite();
      await connect;
    });

    expect(api.flushCloudProviders).toHaveBeenCalledWith(
      expect.arrayContaining([
        expect.objectContaining({ slug: 'openai', ca_cert_pem: 'new CA PEM' }),
      ])
    );
    expect(persist).toHaveBeenCalledWith(
      expect.objectContaining({
        cloudProviders: expect.arrayContaining([
          expect.objectContaining({ caCertPem: 'new CA PEM' }),
        ]),
      })
    );
  });

  it('does not write a provider key when the provider flush fails', async () => {
    api.flushCloudProviders
      .mockRejectedValueOnce(new Error('settings write failed'))
      .mockResolvedValueOnce(undefined);
    const persist = vi.fn().mockResolvedValue(undefined);
    const { result } = renderHook(() =>
      useProviderConnect({
        draft: EMPTY_SETTINGS,
        saved: EMPTY_SETTINGS,
        persist,
        t: key => key,
        onConnected: vi.fn(),
      })
    );

    await act(async () => {
      await expect(
        result.current.connectProvider({
          slug: 'openai',
          value: 'new-key',
          credentialMode: 'api_key',
        })
      ).rejects.toThrow('settings write failed');
    });

    expect(api.flushCloudProviders).toHaveBeenCalledTimes(2);
    expect(api.flushCloudProviders).toHaveBeenNthCalledWith(
      1,
      expect.arrayContaining([expect.objectContaining({ slug: 'openai' })])
    );
    expect(api.flushCloudProviders).toHaveBeenNthCalledWith(2, []);
    expect(api.setCloudProviderKey).not.toHaveBeenCalled();
    expect(persist).not.toHaveBeenCalled();
  });

  it('restores the original provider list when storing the key fails', async () => {
    const existing = {
      id: 'provider-1',
      slug: 'openai',
      label: 'OpenAI',
      endpoint: 'https://api.openai.com/v1',
      authStyle: 'bearer' as const,
      maskedKey: '••••old',
      caCertPem: 'saved CA PEM',
    };
    const settings: AISettings = { ...EMPTY_SETTINGS, cloudProviders: [existing] };
    api.setCloudProviderKey.mockRejectedValueOnce(new Error('key store failed'));
    const persist = vi.fn().mockResolvedValue(undefined);
    const { result } = renderHook(() =>
      useProviderConnect({
        draft: settings,
        saved: settings,
        persist,
        t: key => key,
        onConnected: vi.fn(),
      })
    );

    await act(async () => {
      await expect(
        result.current.connectProvider({
          slug: 'openai',
          value: 'replacement-key',
          credentialMode: 'api_key',
        })
      ).rejects.toThrow('key store failed');
    });

    expect(api.flushCloudProviders).toHaveBeenNthCalledWith(2, [
      {
        id: 'provider-1',
        slug: 'openai',
        label: 'OpenAI',
        endpoint: 'https://api.openai.com/v1',
        ca_cert_pem: 'saved CA PEM',
        auth_style: 'bearer',
      },
    ]);
    expect(persist).not.toHaveBeenCalled();
  });

  it('persists only the latest overlapping connection update', async () => {
    let finishFirst!: () => void;
    let finishSecond!: () => void;
    api.setCloudProviderKey
      .mockImplementationOnce(() => new Promise<void>(resolve => (finishFirst = resolve)))
      .mockImplementationOnce(() => new Promise<void>(resolve => (finishSecond = resolve)));
    const persist = vi.fn().mockResolvedValue(undefined);
    const { result } = renderHook(() =>
      useProviderConnect({
        draft: EMPTY_SETTINGS,
        saved: EMPTY_SETTINGS,
        persist,
        t: key => key,
        onConnected: vi.fn(),
      })
    );

    let first!: Promise<void>;
    act(() => {
      first = result.current.connectProvider({
        slug: 'openai',
        value: 'first-key',
        credentialMode: 'api_key',
      });
    });
    await waitFor(() => expect(finishFirst).toBeTypeOf('function'));

    let second!: Promise<void>;
    act(() => {
      second = result.current.connectProvider({
        slug: 'openai',
        value: 'second-key',
        credentialMode: 'api_key',
      });
    });
    expect(api.setCloudProviderKey).toHaveBeenCalledTimes(1);

    await act(async () => {
      finishFirst();
      await first;
      await waitFor(() => expect(finishSecond).toBeTypeOf('function'));
      finishSecond();
      await second;
    });

    expect(api.setCloudProviderKey.mock.calls.map(([, key]) => key)).toEqual([
      'first-key',
      'second-key',
    ]);
    expect(persist).toHaveBeenCalledTimes(1);
    expect(persist.mock.calls[0][0].cloudProviders[0].slug).toBe('openai');
  });

  it('serializes provider-list updates across different providers', async () => {
    let finishFirst!: () => void;
    api.setCloudProviderKey.mockImplementationOnce(
      () => new Promise<void>(resolve => (finishFirst = resolve))
    );
    const persist = vi.fn().mockResolvedValue(undefined);
    const { result } = renderHook(() =>
      useProviderConnect({
        draft: EMPTY_SETTINGS,
        saved: EMPTY_SETTINGS,
        persist,
        t: key => key,
        onConnected: vi.fn(),
      })
    );

    let first!: Promise<void>;
    act(() => {
      first = result.current.connectProvider({
        slug: 'openai',
        value: 'first-key',
        credentialMode: 'api_key',
      });
    });
    await waitFor(() => expect(finishFirst).toBeTypeOf('function'));

    let second!: Promise<void>;
    act(() => {
      second = result.current.connectProvider({
        slug: 'anthropic',
        value: 'second-key',
        credentialMode: 'api_key',
      });
    });
    expect(api.setCloudProviderKey).toHaveBeenCalledTimes(1);

    await act(async () => {
      finishFirst();
      await first;
      await second;
    });

    expect(api.setCloudProviderKey.mock.calls.map(([, key]) => key)).toEqual([
      'first-key',
      'second-key',
    ]);
    expect(persist).toHaveBeenCalledTimes(2);
    expect(
      persist.mock.calls[1][0].cloudProviders.map((provider: { slug: string }) => provider.slug)
    ).toEqual(['openai', 'anthropic']);
    expect(api.flushCloudProviders).toHaveBeenNthCalledWith(
      2,
      expect.arrayContaining([
        expect.objectContaining({ slug: 'openai' }),
        expect.objectContaining({ slug: 'anthropic' }),
      ])
    );
  });
});
