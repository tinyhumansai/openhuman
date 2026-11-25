import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { type AISettings, EMPTY_SETTINGS } from '../aiPanelTypes';
import { useProviderConnect } from '../useProviderConnect';

const api = vi.hoisted(() => ({
  clearCloudProviderKey: vi.fn(),
  flushCloudProviders: vi.fn(),
  listProviderModels: vi.fn(),
  loadProviderAuthErrors: vi.fn(),
  setCloudProviderKey: vi.fn(),
}));

vi.mock('../../../../../services/api/aiSettingsApi', async importOriginal => ({
  ...(await importOriginal<typeof import('../../../../../services/api/aiSettingsApi')>()),
  clearCloudProviderKey: api.clearCloudProviderKey,
  flushCloudProviders: api.flushCloudProviders,
  listProviderModels: api.listProviderModels,
  loadProviderAuthErrors: api.loadProviderAuthErrors,
  setCloudProviderKey: api.setCloudProviderKey,
}));

beforeEach(() => {
  api.clearCloudProviderKey.mockReset();
  api.clearCloudProviderKey.mockResolvedValue(undefined);
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

  describe('OCI Generative AI', () => {
    const ociProvider = (maskedKey: string) => ({
      id: 'p_oci',
      slug: 'oci',
      label: 'OCI Generative AI',
      endpoint: 'https://inference.generativeai.us-chicago-1.oci.oraclecloud.com',
      authStyle: 'oci' as const,
      maskedKey,
      oci: {
        auth_mode: 'api_key' as const,
        transport: 'openai' as const,
        region: 'us-chicago-1',
        project_id: 'p',
      },
    });
    const renderConnect = (settings: AISettings, persist = vi.fn().mockResolvedValue(undefined)) =>
      renderHook(() =>
        useProviderConnect({
          draft: settings,
          saved: settings,
          persist,
          t: key => key,
          onConnected: vi.fn(),
        })
      );

    it('stores the API key after the flush and skips the /models probe', async () => {
      const persist = vi.fn().mockResolvedValue(undefined);
      const { result } = renderConnect(EMPTY_SETTINGS, persist);
      const oci = {
        auth_mode: 'api_key' as const,
        transport: 'openai' as const,
        region: 'us-chicago-1',
      };

      await act(async () => {
        await result.current.connectProvider({
          slug: 'oci',
          value: ' genai-key ',
          credentialMode: 'oci',
          oci,
        });
      });

      expect(api.flushCloudProviders).toHaveBeenCalledWith([
        expect.objectContaining({ slug: 'oci', auth_style: 'oci', oci }),
      ]);
      expect(api.setCloudProviderKey).toHaveBeenCalledWith('oci', 'genai-key');
      expect(api.flushCloudProviders.mock.invocationCallOrder[0]).toBeLessThan(
        api.setCloudProviderKey.mock.invocationCallOrder[0]
      );
      expect(api.listProviderModels).not.toHaveBeenCalled();
      expect(api.clearCloudProviderKey).not.toHaveBeenCalled();
      expect(persist).toHaveBeenCalledWith(
        expect.objectContaining({ cloudProviders: [expect.objectContaining({ slug: 'oci', oci })] })
      );
    });

    it('keeps the stored API key when an edit leaves the field blank', async () => {
      const settings: AISettings = { ...EMPTY_SETTINGS, cloudProviders: [ociProvider('••••key')] };
      const persist = vi.fn().mockResolvedValue(undefined);
      const { result } = renderConnect(settings, persist);

      await act(async () => {
        await result.current.connectProvider({
          slug: 'oci',
          value: '',
          credentialMode: 'oci',
          oci: {
            auth_mode: 'api_key',
            transport: 'actions',
            region: 'us-chicago-1',
            compartment_id: 'c',
          },
        });
      });

      expect(api.setCloudProviderKey).not.toHaveBeenCalled();
      const saved = persist.mock.calls[0][0].cloudProviders[0];
      expect(saved.maskedKey.startsWith('••••')).toBe(true);
      expect(saved.oci.transport).toBe('actions');
    });

    it('drops a stale API key when switching to a signed mode', async () => {
      const settings: AISettings = { ...EMPTY_SETTINGS, cloudProviders: [ociProvider('••••key')] };
      const { result } = renderConnect(settings);

      await act(async () => {
        await result.current.connectProvider({
          slug: 'oci',
          value: '',
          credentialMode: 'oci',
          oci: {
            auth_mode: 'session_token',
            transport: 'openai',
            profile: 'SESSION',
            compartment_id: 'c',
          },
        });
      });

      expect(api.clearCloudProviderKey).toHaveBeenCalledWith('oci');
      expect(api.setCloudProviderKey).not.toHaveBeenCalled();
      expect(api.listProviderModels).not.toHaveBeenCalled();
    });

    it('aborts the switch and restores the providers when the key cannot be deleted', async () => {
      const settings: AISettings = { ...EMPTY_SETTINGS, cloudProviders: [ociProvider('••••key')] };
      const persist = vi.fn().mockResolvedValue(undefined);
      api.clearCloudProviderKey.mockRejectedValueOnce(new Error('keyring locked'));
      const { result } = renderConnect(settings, persist);

      await expect(
        act(async () => {
          await result.current.connectProvider({
            slug: 'oci',
            value: '',
            credentialMode: 'oci',
            oci: {
              auth_mode: 'config_profile',
              transport: 'openai',
              compartment_id: 'c',
              project_id: 'p',
            },
          });
        })
      ).rejects.toThrow('keyring locked');

      expect(api.flushCloudProviders).toHaveBeenLastCalledWith([
        expect.objectContaining({
          slug: 'oci',
          oci: expect.objectContaining({ auth_mode: 'api_key' }),
        }),
      ]);
      expect(persist).not.toHaveBeenCalled();
    });

    it('persists a blank endpoint so the core routes by region, keeping explicit overrides', async () => {
      const oci = {
        auth_mode: 'api_key' as const,
        transport: 'openai' as const,
        region: 'eu-frankfurt-1',
        project_id: 'p',
      };
      const { result } = renderConnect({
        ...EMPTY_SETTINGS,
        cloudProviders: [ociProvider('••••key')],
      });
      await act(async () => {
        await result.current.connectProvider({
          slug: 'oci',
          value: '',
          credentialMode: 'oci',
          oci,
        });
      });
      expect(api.flushCloudProviders).toHaveBeenLastCalledWith([
        expect.objectContaining({ slug: 'oci', endpoint: '' }),
      ]);

      const custom = {
        ...ociProvider('••••key'),
        id: 'p_oci-private',
        slug: 'oci-private',
        label: 'OCI private',
        endpoint: 'https://genai.private.example.oraclecloud.com',
      };
      const { result: second } = renderConnect({ ...EMPTY_SETTINGS, cloudProviders: [custom] });
      await act(async () => {
        await second.current.connectProvider({
          slug: 'oci-private',
          value: '',
          credentialMode: 'oci',
          oci,
        });
      });
      expect(api.flushCloudProviders).toHaveBeenLastCalledWith([
        expect.objectContaining({
          slug: 'oci-private',
          label: 'OCI private',
          auth_style: 'oci',
          endpoint: 'https://genai.private.example.oraclecloud.com',
        }),
      ]);
    });

    it('rejects a connection without OCI settings and leaves the queue usable', async () => {
      const { result } = renderConnect(EMPTY_SETTINGS);

      await expect(
        result.current.connectProvider({ slug: 'oci', value: '', credentialMode: 'oci' })
      ).rejects.toThrow('OCI settings are required');

      await act(async () => {
        await result.current.connectProvider({
          slug: 'openai',
          value: 'sk-test',
          credentialMode: 'api_key',
        });
      });
      expect(api.setCloudProviderKey).toHaveBeenCalledWith('openai', 'sk-test');
    });
  });
});
