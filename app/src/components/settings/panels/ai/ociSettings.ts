/*
 * Pure helpers for the OCI Generative AI provider form: option tables, the
 * validation that mirrors the Rust factory (`inference::provider::factory::oci`),
 * normalisation of the payload sent to the core, and the one-line row summary.
 *
 * Kept free of React so the rules are unit-testable on their own. The core is
 * the authority; these checks only catch the same mistakes before a round trip.
 */
import type {
  OciAuthMode,
  OciSettings,
  OciTransport,
} from '../../../../utils/tauriCommands/config';
import { defaultEndpointForBuiltinCloudProvider } from '../builtinCloudProviders';

export const OCI_PROVIDER_SLUG = 'oci';

export const OCI_AUTH_MODE_OPTIONS: { value: OciAuthMode; labelKey: string }[] = [
  { value: 'api_key', labelKey: 'settings.ai.oci.mode.apiKey' },
  { value: 'config_profile', labelKey: 'settings.ai.oci.mode.configProfile' },
  { value: 'session_token', labelKey: 'settings.ai.oci.mode.sessionToken' },
  { value: 'resource_principal', labelKey: 'settings.ai.oci.mode.resourcePrincipal' },
  { value: 'instance_principal', labelKey: 'settings.ai.oci.mode.instancePrincipal' },
  { value: 'oke_workload_identity', labelKey: 'settings.ai.oci.mode.okeWorkloadIdentity' },
];

export const OCI_TRANSPORT_OPTIONS: { value: OciTransport; labelKey: string }[] = [
  { value: 'openai', labelKey: 'settings.ai.oci.transport.openai' },
  { value: 'actions', labelKey: 'settings.ai.oci.transport.actions' },
  { value: 'native_generic', labelKey: 'settings.ai.oci.transport.nativeGeneric' },
  { value: 'native_cohere', labelKey: 'settings.ai.oci.transport.nativeCohere' },
];

export const DEFAULT_OCI_SETTINGS: OciSettings = { auth_mode: 'api_key', transport: 'openai' };

/** Modes whose key material comes from an OCI config-file profile. */
export const isOciProfileMode = (mode: OciAuthMode): boolean =>
  mode === 'config_profile' || mode === 'session_token';

/** Modes that identify the workload itself and so store no secret. */
export const isOciPrincipalMode = (mode: OciAuthMode): boolean =>
  mode === 'resource_principal' ||
  mode === 'instance_principal' ||
  mode === 'oke_workload_identity';

/** Principal modes that only exist in a build with the `oci-principals` gate. */
export const isOciGatedPrincipalMode = (mode: OciAuthMode): boolean =>
  mode === 'instance_principal' || mode === 'oke_workload_identity';

/**
 * Whether the entry must name a compartment. A Generative AI API key carries
 * its own, so only `openai` with `api_key` can omit it; IAM-signed calls need
 * one on every transport. Mirrors the core's check.
 */
export const ociNeedsCompartment = (settings: OciSettings): boolean =>
  settings.transport !== 'openai' || settings.auth_mode !== 'api_key';

/**
 * Only `/openai/v1` carries the Generative AI project header, and it requires
 * one. Mirrors the core's check.
 */
export const ociTransportUsesProject = (transport: OciTransport): boolean => transport === 'openai';

/**
 * The `endpoint` to persist on an OCI entry: an explicit override (another
 * realm, a private endpoint) is kept, while a blank value or the built-in
 * preset's display URL becomes blank so the core derives the host from the
 * region.
 */
export function ociEndpointOverride(existing: string | undefined): string {
  const endpoint = trimmed(existing).replace(/\/+$/, '');
  const preset = defaultEndpointForBuiltinCloudProvider(OCI_PROVIDER_SLUG).replace(/\/+$/, '');
  return endpoint === preset ? '' : endpoint;
}

type OptionalOciField =
  | 'config_file'
  | 'profile'
  | 'region'
  | 'compartment_id'
  | 'project_id'
  | 'dedicated_endpoint_id';

const trimmed = (value: string | undefined): string => (value ?? '').trim();

/**
 * First problem with the form as an i18n key, or `null` when it can be saved.
 * `hasStoredKey` is true when an API key already sits in the keyring, so an
 * edit that leaves the key field blank keeps it.
 */
export function validateOciSettings(
  settings: OciSettings,
  apiKey: string,
  hasStoredKey: boolean
): string | null {
  // A profile or principal can supply the region; only api_key mode cannot.
  if (settings.auth_mode === 'api_key' && !trimmed(settings.region)) {
    return 'settings.ai.oci.regionRequired';
  }
  if (settings.auth_mode === 'api_key' && !trimmed(apiKey) && !hasStoredKey) {
    return 'settings.ai.apiKeyRequired';
  }
  if (ociNeedsCompartment(settings) && !trimmed(settings.compartment_id)) {
    return 'settings.ai.oci.compartmentRequired';
  }
  if (ociTransportUsesProject(settings.transport) && !trimmed(settings.project_id)) {
    return 'settings.ai.oci.projectRequired';
  }
  return null;
}

/**
 * The payload persisted on the provider entry: values trimmed, empty optional
 * fields dropped, and fields that do not apply to the chosen mode / transport
 * removed so a stale value cannot ride along after the user switches.
 */
export function normalizeOciSettings(settings: OciSettings): OciSettings {
  const out: OciSettings = { auth_mode: settings.auth_mode, transport: settings.transport };
  const set = (key: OptionalOciField) => {
    const value = trimmed(settings[key]);
    if (value) out[key] = value;
  };
  if (isOciProfileMode(settings.auth_mode)) {
    set('config_file');
    set('profile');
  }
  set('region');
  set('compartment_id');
  if (ociTransportUsesProject(settings.transport)) set('project_id');
  set('dedicated_endpoint_id');
  return out;
}

/** `region · mode` for the connected-provider row; empty when nothing is set. */
export function ociSettingsSummary(
  settings: OciSettings | undefined,
  t: (key: string) => string
): string {
  if (!settings) return '';
  const mode = OCI_AUTH_MODE_OPTIONS.find(option => option.value === settings.auth_mode);
  return [trimmed(settings.region), mode ? t(mode.labelKey) : ''].filter(Boolean).join(' · ');
}
