import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { EMPTY_SETTINGS } from '../aiPanelTypes';
import { useCloudProviderEditorSubmit } from '../useCloudProviderEditorSubmit';

const api = vi.hoisted(() => ({
  flushCloudProviders: vi.fn(),
  listProviderModels: vi.fn(),
  setCloudProviderKey: vi.fn(),
}));

vi.mock('../../../../../services/api/aiSettingsApi', async importOriginal => ({
  ...(await importOriginal<typeof import('../../../../../services/api/aiSettingsApi')>()),
  flushCloudProviders: api.flushCloudProviders,
  listProviderModels: api.listProviderModels,
  setCloudProviderKey: api.setCloudProviderKey,
}));

beforeEach(() => {
  api.flushCloudProviders.mockReset();
  api.listProviderModels.mockReset();
  api.setCloudProviderKey.mockReset();
  api.setCloudProviderKey.mockResolvedValue(undefined);
});

describe('useCloudProviderEditorSubmit', () => {
  it('validates the provider CA before persisting a new key', async () => {
    api.flushCloudProviders.mockRejectedValue(new Error('invalid CA bundle'));
    const { result } = renderHook(() =>
      useCloudProviderEditorSubmit({
        editing: 'new',
        draft: EMPTY_SETTINGS,
        saved: EMPTY_SETTINGS,
        persist: vi.fn(),
        t: key => key,
        onDone: vi.fn(),
      })
    );

    await act(async () => {
      await expect(
        result.current(
          {
            id: '',
            slug: 'private-provider',
            label: 'Private provider',
            endpoint: 'https://provider.example/v1',
            authStyle: 'bearer',
            maskedKey: '',
            caCertPem: 'invalid PEM',
          },
          'new-secret'
        )
      ).rejects.toThrow('invalid CA bundle');
    });

    expect(api.flushCloudProviders).toHaveBeenCalledTimes(2);
    expect(api.flushCloudProviders).toHaveBeenCalledWith(
      expect.arrayContaining([
        expect.objectContaining({ slug: 'private-provider', ca_cert_pem: 'invalid PEM' }),
      ])
    );
    expect(api.flushCloudProviders).toHaveBeenNthCalledWith(2, []);
    expect(api.setCloudProviderKey).not.toHaveBeenCalled();
    expect(api.listProviderModels).not.toHaveBeenCalled();
  });

  it('restores the previous provider list when key storage fails', async () => {
    api.flushCloudProviders.mockResolvedValue(undefined);
    api.flushCloudProviders.mockResolvedValueOnce(undefined).mockResolvedValueOnce(undefined);
    api.setCloudProviderKey.mockRejectedValue(new Error('keyring locked'));
    const previous = {
      id: 'old-provider',
      slug: 'private-provider',
      label: 'Old provider',
      endpoint: 'https://old.example/v1',
      authStyle: 'bearer' as const,
      maskedKey: '••••old',
      caCertPem: 'old CA',
    };
    const saved = { ...EMPTY_SETTINGS, cloudProviders: [previous] };
    const { result } = renderHook(() =>
      useCloudProviderEditorSubmit({
        editing: previous,
        draft: saved,
        saved,
        persist: vi.fn(),
        t: key => key,
        onDone: vi.fn(),
      })
    );

    await act(async () => {
      await expect(
        result.current(
          { ...previous, endpoint: 'https://new.example/v1', caCertPem: 'new CA' },
          'replacement-key'
        )
      ).rejects.toThrow('keyring locked');
    });

    expect(api.flushCloudProviders).toHaveBeenCalledTimes(2);
    expect(api.flushCloudProviders).toHaveBeenNthCalledWith(
      1,
      expect.arrayContaining([
        expect.objectContaining({ endpoint: 'https://new.example/v1', ca_cert_pem: 'new CA' }),
      ])
    );
    expect(api.flushCloudProviders).toHaveBeenNthCalledWith(
      2,
      expect.arrayContaining([
        expect.objectContaining({ endpoint: 'https://old.example/v1', ca_cert_pem: 'old CA' }),
      ])
    );
    expect(api.listProviderModels).not.toHaveBeenCalled();
  });

  it('restores the previous provider list when the eager flush fails', async () => {
    api.flushCloudProviders
      .mockRejectedValueOnce(new Error('settings write failed'))
      .mockResolvedValueOnce(undefined);
    const previous = {
      id: 'old-provider',
      slug: 'private-provider',
      label: 'Old provider',
      endpoint: 'https://old.example/v1',
      authStyle: 'bearer' as const,
      maskedKey: '••••old',
      caCertPem: 'old CA',
    };
    const saved = { ...EMPTY_SETTINGS, cloudProviders: [previous] };
    const { result } = renderHook(() =>
      useCloudProviderEditorSubmit({
        editing: previous,
        draft: saved,
        saved,
        persist: vi.fn(),
        t: key => key,
        onDone: vi.fn(),
      })
    );

    await act(async () => {
      await expect(
        result.current(
          { ...previous, endpoint: 'https://new.example/v1', caCertPem: 'new CA' },
          'replacement-key'
        )
      ).rejects.toThrow('settings write failed');
    });

    expect(api.flushCloudProviders).toHaveBeenCalledTimes(2);
    expect(api.flushCloudProviders).toHaveBeenNthCalledWith(
      2,
      expect.arrayContaining([
        expect.objectContaining({ endpoint: 'https://old.example/v1', ca_cert_pem: 'old CA' }),
      ])
    );
    expect(api.setCloudProviderKey).not.toHaveBeenCalled();
    expect(api.listProviderModels).not.toHaveBeenCalled();
  });
});
