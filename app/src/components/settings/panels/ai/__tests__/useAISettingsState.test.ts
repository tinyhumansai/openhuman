/**
 * `useAISettings().save` re-probes every new or re-pointed provider through
 * `openhuman.inference_list_models` before persisting. A CLI-login provider
 * (`claude-code`, endpoint `cli://claude-code`) has no `/models` endpoint, so
 * probing it always failed with a core error and blocked the save — the same
 * reason `useProviderConnect` already skips it at add-time (`isCliLogin`).
 */
import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { EMPTY_SETTINGS } from '../aiPanelTypes';
import { useAISettings } from '../useAISettingsState';

const api = vi.hoisted(() => ({
  listProviderModels: vi.fn(),
  loadAISettings: vi.fn(),
  saveAISettings: vi.fn(),
  flushCloudProviders: vi.fn(),
}));

vi.mock('../../../../../services/api/aiSettingsApi', async importOriginal => ({
  ...(await importOriginal<typeof import('../../../../../services/api/aiSettingsApi')>()),
  listProviderModels: api.listProviderModels,
  loadAISettings: api.loadAISettings,
  saveAISettings: api.saveAISettings,
  flushCloudProviders: api.flushCloudProviders,
}));

beforeEach(() => {
  api.listProviderModels.mockReset();
  api.listProviderModels.mockRejectedValue(new Error('core: no /models for cli provider'));
  api.loadAISettings.mockReset();
  api.loadAISettings.mockResolvedValue({
    cloudProviders: [],
    routing: EMPTY_SETTINGS.routing,
    modelRegistry: [],
    defaultModel: '',
  });
  api.saveAISettings.mockReset();
  api.saveAISettings.mockResolvedValue(undefined);
  api.flushCloudProviders.mockReset();
  api.flushCloudProviders.mockResolvedValue(undefined);
});

async function mountLoaded() {
  const hook = renderHook(() => useAISettings());
  await waitFor(() => expect(hook.result.current.loading).toBe(false));
  return hook;
}

describe('useAISettings().save — save-time provider re-probe', () => {
  it('does not probe a claude-code (CLI login) provider', async () => {
    const { result } = await mountLoaded();

    act(() => {
      result.current.setDraft(prev => ({
        ...prev,
        cloudProviders: [
          {
            id: 'p-claude-code',
            slug: 'claude-code',
            label: 'Claude Code',
            endpoint: 'cli://claude-code',
            authStyle: 'bearer',
            maskedKey: '',
          },
        ],
      }));
    });

    let ok = false;
    await act(async () => {
      ok = await result.current.save();
    });

    expect(api.listProviderModels).not.toHaveBeenCalled();
    expect(ok).toBe(true);
    expect(api.saveAISettings).toHaveBeenCalledTimes(1);
  });

  it('still probes an ordinary new provider and refuses to save when it fails', async () => {
    const { result } = await mountLoaded();

    act(() => {
      result.current.setDraft(prev => ({
        ...prev,
        cloudProviders: [
          {
            id: 'p-openai',
            slug: 'openai',
            label: 'OpenAI',
            endpoint: 'https://api.openai.com/v1',
            authStyle: 'bearer',
            maskedKey: '••••abcd',
          },
        ],
      }));
    });

    let ok = true;
    await act(async () => {
      ok = await result.current.save();
    });

    expect(api.listProviderModels).toHaveBeenCalledWith('openai');
    expect(ok).toBe(false);
    expect(api.saveAISettings).not.toHaveBeenCalled();
  });
});
