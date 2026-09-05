import {
  getRegistryTabForDetailKind,
  isDetailAllowedForTab,
  isRegistryDetailKind,
  isRegistryTab,
  REGISTRY_KEY_MAX_LENGTH,
  type RegistryDetailRef,
  type RegistryUrlState,
} from './types';

export { REGISTRY_KEY_MAX_LENGTH } from './types';

function parsePositiveBase10Integer(value: string | null): number | null {
  if (!value || !/^\d+$/.test(value)) {
    return null;
  }

  const parsed = Number.parseInt(value, 10);
  return Number.isSafeInteger(parsed) && parsed > 0 ? parsed : null;
}

function parseDetail(
  params: URLSearchParams,
  tab: RegistryUrlState['tab']
): RegistryDetailRef | null {
  const kindValue = params.get('kind');
  const key = params.get('key');
  const version = parsePositiveBase10Integer(params.get('version'));

  if (!kindValue || !isRegistryDetailKind(kindValue) || !isDetailAllowedForTab(tab, kindValue)) {
    return null;
  }

  if (!key || key.length > REGISTRY_KEY_MAX_LENGTH || version === null) {
    return null;
  }

  return { kind: kindValue, key, version };
}

export function parseRegistryUrlState(search: string): RegistryUrlState {
  const params = new URLSearchParams(search.startsWith('?') ? search.slice(1) : search);
  const tab = isRegistryTab(params.get('tab') ?? '')
    ? (params.get('tab') as RegistryUrlState['tab'])
    : 'agents';

  return { tab, detail: parseDetail(params, tab) };
}

export function serializeRegistryUrlState(state: RegistryUrlState): string {
  const params = new URLSearchParams();
  params.set('tab', state.tab);

  if (state.detail && getRegistryTabForDetailKind(state.detail.kind) === state.tab) {
    params.set('kind', state.detail.kind);
    params.set('key', state.detail.key);
    params.set('version', String(state.detail.version));
  }

  return params.toString();
}
