import { describe, expect, it } from 'vitest';

import {
  parseRegistryUrlState,
  REGISTRY_KEY_MAX_LENGTH,
  serializeRegistryUrlState,
} from './urlState';

describe('registry URL state', () => {
  it('defaults to the Agents tab with no selection', () => {
    expect(parseRegistryUrlState('')).toEqual({ tab: 'agents', detail: null });
    expect(parseRegistryUrlState('?ignored=value')).toEqual({ tab: 'agents', detail: null });
  });

  it('parses and canonicalizes closed tab/kind/key/version identity', () => {
    const state = parseRegistryUrlState(
      '?tab=tools&kind=tool-definition&key=tool.alpha&version=0007&cursor=secret&observedAt=secret&payload=secret'
    );

    expect(state).toEqual({
      tab: 'tools',
      detail: { kind: 'tool-definition', key: 'tool.alpha', version: 7 },
    });
    expect(serializeRegistryUrlState(state)).toBe(
      'tab=tools&kind=tool-definition&key=tool.alpha&version=7'
    );
  });

  it('drops invalid or cross-tab detail selectors as a unit', () => {
    expect(parseRegistryUrlState('?tab=connectors&kind=agent&key=agent.alpha&version=7')).toEqual({
      tab: 'connectors',
      detail: null,
    });

    expect(
      parseRegistryUrlState(
        `?tab=agents&kind=agent&key=${'a'.repeat(REGISTRY_KEY_MAX_LENGTH + 1)}&version=7`
      )
    ).toEqual({ tab: 'agents', detail: null });

    expect(parseRegistryUrlState('?tab=agents&kind=agent&key=agent.alpha&version=0')).toEqual({
      tab: 'agents',
      detail: null,
    });

    expect(parseRegistryUrlState('?tab=agents&kind=agent&key=agent.alpha')).toEqual({
      tab: 'agents',
      detail: null,
    });
  });
});
