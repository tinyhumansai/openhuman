/**
 * Azure AI Foundry deployment-name helpers (issue #5213).
 *
 * Azure separates the **base model id** it was deployed from
 * (`gpt-5.6-terra-2026-07-09`) from the user-chosen **deployment name**
 * (`gpt-5.6-terra`) that actually routes the request. The OpenAI-compatible
 * surface keys the request body's `model` field on the *deployment name*, so a
 * value taken from the provider's `/models` catalog (which lists base model
 * ids) yields "Model not found".
 *
 * OpenHuman's routing already sends the `<model>` half of a `"<slug>:<model>"`
 * provider string verbatim as that body field, so nothing in the request path
 * needs to change. The only defect is that the settings UI sourced the value
 * exclusively from the `/models` catalog, leaving no way to type a deployment
 * name that is not in it. These helpers let the AI panel recognise an Azure
 * connection and switch the model field to free text.
 *
 * Detection is by endpoint **host**, not slug: Azure is reachable today only
 * through the generic "Add cloud provider" flow, so the user picks the slug
 * (`azure`, `azure-foundry`, `my-azure`, …) and the host is the one stable
 * signal. Every Azure resource has its own subdomain under a small set of
 * Microsoft-owned parents.
 */

/**
 * Authority hosts (or parent domains) that serve Azure AI Foundry / Azure
 * OpenAI inference. A host matches when it equals one of these or is a
 * subdomain of it, which is how per-resource endpoints like
 * `my-resource.openai.azure.com` are recognised.
 */
const AZURE_ENDPOINT_HOSTS = [
  // The two hosts Microsoft documents for the OpenAI-compatible v1 base URL
  // (`https://<resource>.<host>/openai/v1/`), plus the resource host the older
  // `api-version` surface is served from.
  'openai.azure.com',
  'services.ai.azure.com',
  'cognitiveservices.azure.com',
  // NOT `inference.ai.azure.com` / `models.ai.azure.com`. Those are the Foundry
  // *serverless* endpoints, which speak the Azure AI Model Inference API at
  // `{endpoint}/models/chat/completions` and key the `model` field on the model
  // name, not on a deployment name. Classifying them here would relabel a
  // correct model id as a "deployment name" and mislead the user in the one
  // place this module exists to make clear.
  // Sovereign clouds. Azure OpenAI is offered in Azure Government
  // (`*.openai.azure.us`) and in Azure operated by 21Vianet / China
  // (`*.openai.azure.cn`). They are separate DNS parents, so the commercial
  // `.com` entries above do not cover them, and without these a sovereign
  // tenant falls through to the exact "model not found" path this module
  // exists to prevent.
  'openai.azure.us',
  'openai.azure.cn',
] as const;

/**
 * Extract the lowercased authority host from an endpoint URL, dropping the
 * scheme, any userinfo, the port and the path. Returns an empty string when no
 * host can be parsed.
 *
 * Mirrors the Rust `endpoint_host` helper in
 * `src/openhuman/config/schema/cloud_providers.rs` — including its tolerance
 * for a missing scheme — so both sides classify a stored endpoint identically.
 */
export function endpointHost(endpoint: string | null | undefined): string {
  const trimmed = (endpoint ?? '').trim().toLowerCase();
  if (!trimmed) return '';
  // Drop the scheme (`https://…`); tolerate a bare `host/path` form.
  const schemeIdx = trimmed.indexOf('://');
  const afterScheme = schemeIdx === -1 ? trimmed : trimmed.slice(schemeIdx + 3);
  // The authority ends at the first path / query / fragment delimiter.
  const authority = afterScheme.split(/[/?#]/)[0] ?? '';
  // Strip any `user:pass@` userinfo prefix.
  const atIdx = authority.lastIndexOf('@');
  const hostPort = atIdx === -1 ? authority : authority.slice(atIdx + 1);
  // Strip the port, handling bracketed IPv6 literals (`[::1]:8080`).
  if (hostPort.startsWith('[')) {
    const close = hostPort.indexOf(']');
    return close === -1 ? hostPort.slice(1) : hostPort.slice(1, close);
  }
  const colonIdx = hostPort.lastIndexOf(':');
  return (colonIdx === -1 ? hostPort : hostPort.slice(0, colonIdx)).trim();
}

/**
 * Whether `endpoint` points at an Azure AI Foundry / Azure OpenAI resource,
 * i.e. a provider whose `model` field must carry a deployment name.
 */
export function isAzureFoundryEndpoint(endpoint: string | null | undefined): boolean {
  const host = endpointHost(endpoint);
  if (!host) return false;
  return AZURE_ENDPOINT_HOSTS.some(known => host === known || host.endsWith(`.${known}`));
}

/**
 * Whether an Azure endpoint points at the OpenAI-compatible **v1** base
 * (`https://<resource>.<host>/openai/v1[/]`).
 *
 * This matters because only that base behaves like every other provider
 * OpenHuman stores: it serves a `GET {base}/models` listing and accepts the
 * resource key in the `authorization` header (Azure's published v1 spec
 * declares both an `api-key` and an `authorization` API-key scheme), which is
 * the `bearer` auth style custom providers are created with. The older
 * `api-version` surface serves neither, so a bare resource URL copied out of
 * the portal fails the probe and then fails inference.
 *
 * Returns `false` for a non-Azure endpoint — callers pair it with
 * {@link isAzureFoundryEndpoint}.
 */
export function isAzureV1BaseUrl(endpoint: string | null | undefined): boolean {
  if (!isAzureFoundryEndpoint(endpoint)) return false;
  const trimmed = (endpoint ?? '').trim().toLowerCase();
  const schemeIdx = trimmed.indexOf('://');
  const afterScheme = schemeIdx === -1 ? trimmed : trimmed.slice(schemeIdx + 3);
  const slashIdx = afterScheme.indexOf('/');
  // No path at all (a bare host) is not the v1 base.
  const path = slashIdx === -1 ? '' : afterScheme.slice(slashIdx);
  return /^\/openai\/v1\/?$/.test(path);
}

/**
 * Whether a stored model value carries the fingerprint of a **pre-fix** Azure
 * selection: it is exactly an entry from the provider's `/models` catalog.
 *
 * Before this fix the dropdown was the only way to set the value, so catalog
 * membership is precisely the signature of a connection configured the broken
 * way. It stays a *hint* rather than an error because a user is free to name a
 * deployment after its base model, in which case the value is already correct
 * and confirming it is a no-op. Nothing is rewritten on the user's behalf.
 */
export function looksLikeAzureBaseModelId(
  model: string | null | undefined,
  catalogModelIds: readonly string[]
): boolean {
  const trimmed = (model ?? '').trim();
  if (!trimmed) return false;
  return catalogModelIds.some(id => id.trim() === trimmed);
}
