/**
 * About-app capability catalog client.
 *
 * Thin wrapper around the `openhuman.about_app_*` JSON-RPC methods exposed by
 * the Rust core (`src/openhuman/about_app/schemas.rs`). The Privacy surface is
 * the first consumer; future panels can reuse the same types.
 */
import { callCoreRpc } from '../../services/coreRpcClient';
import { CommandResponse } from './common';

export type CapabilityCategory =
  | 'conversation'
  | 'intelligence'
  | 'skills'
  | 'local_ai'
  | 'team'
  | 'settings'
  | 'auth'
  | 'screen_intelligence'
  | 'channels'
  | 'automation';

export type CapabilityStatus = 'stable' | 'beta' | 'coming_soon' | 'deprecated';

export type PrivacyDataKind = 'raw' | 'derived' | 'credentials' | 'diagnostics' | 'metadata';

export interface CapabilityPrivacy {
  leaves_device: boolean;
  data_kind: PrivacyDataKind;
  destinations: string[];
}

export interface Capability {
  id: string;
  name: string;
  domain: string;
  category: CapabilityCategory;
  description: string;
  how_to: string;
  status: CapabilityStatus;
  privacy?: CapabilityPrivacy;
}

export async function listCapabilities(category?: CapabilityCategory): Promise<Capability[]> {
  const response = await callCoreRpc<CommandResponse<Capability[]> | Capability[]>({
    method: 'openhuman.about_app_list',
    params: category ? { category } : {},
  });
  // RpcOutcome::single_log emits {result, logs}; bare arrays are handled too
  // for forward-compat if logs ever go away.
  if (Array.isArray(response)) {
    return response;
  }
  return response.result;
}
