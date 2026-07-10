/**
 * Per-toolkit declarative registry for provider-specific required fields the
 * Composio connect flow must collect *before* calling
 * `openhuman.composio_authorize`. Without these fields the backend returns
 * `ConnectedAccount_MissingRequiredFields` (code 612) and the user is left
 * with an unhelpful raw error (#2127, #1702).
 *
 * Adding a new provider-specific field is intentionally a single registry
 * entry — no per-toolkit branches inside `ComposioConnectModal` anymore.
 *
 * Field values are forwarded verbatim into the `extra_params` object of
 * `composio_authorize`; the key on each entry is also the param name the
 * backend expects (e.g. `waba_id`, `subdomain`, `org_name`).
 */
/**
 * String-typed i18n keys (the `useT().t` callback accepts plain strings;
 * see `app/src/lib/i18n/I18nContext.tsx`). Kept as an alias here so the
 * intent — "this is an i18n lookup key, not arbitrary text" — is local to
 * the registry without forcing the rest of the codebase to adopt a strict
 * key union.
 */
type TranslationKey = string;

export interface ToolkitRequiredField {
  /**
   * Field id. Also used verbatim as the `extra_params` key forwarded to
   * `openhuman.composio_authorize`, so it must match exactly what the
   * Composio backend expects for the toolkit.
   */
  key: string;
  /** i18n key for the input label. */
  labelKey: TranslationKey;
  /** Optional i18n key for the hint paragraph rendered below the input. */
  hintKey?: TranslationKey;
  /**
   * Optional i18n key for the placeholder shown when the input is empty.
   * Keyed (not raw text) so the placeholder stays inside the i18n pipeline
   * like `labelKey` / `hintKey` — all UI text goes through `useT()`.
   */
  placeholderKey?: TranslationKey;
  /**
   * Optional fixed suffix rendered inside the input (e.g. `.atlassian.net`).
   * Purely cosmetic — never included in the submitted value.
   */
  suffix?: string;
  /**
   * Validate the trimmed value. Return null when valid, or the i18n key for
   * an inline error message when invalid. When omitted, only the
   * non-empty check (`required`) is enforced.
   */
  validate?: (value: string) => TranslationKey | null;
}

/**
 * Subdomain-style validator shared between Atlassian (`<sub>.atlassian.net`)
 * and Dynamics 365 (`<myorg>.crm.dynamics.com`) — both reject full URLs and
 * accept the short DNS-label form (1-63 chars, alphanumerics + hyphens, no
 * leading/trailing hyphen).
 */
function validateSubdomainLabel(value: string): TranslationKey | null {
  const trimmed = value.trim();
  if (!/^[a-z0-9][a-z0-9-]{0,61}[a-z0-9]$|^[a-z0-9]$/i.test(trimmed)) {
    return 'composio.connect.subdomainInvalid';
  }
  return null;
}

/**
 * Registry of toolkit slug → required-field definitions. Empty list (or
 * absent entry) means the toolkit has no upfront required fields and the
 * existing OAuth-only flow runs unchanged.
 */
export const TOOLKIT_REQUIRED_FIELDS: Readonly<Record<string, readonly ToolkitRequiredField[]>> =
  Object.freeze({
    whatsapp: [
      {
        key: 'waba_id',
        labelKey: 'composio.connect.wabaIdLabel',
        hintKey: 'composio.connect.wabaIdHint',
        placeholderKey: 'composio.connect.wabaIdPlaceholder',
      },
    ],
    jira: [
      {
        key: 'subdomain',
        labelKey: 'composio.connect.atlassianSubdomainLabel',
        hintKey: 'composio.connect.atlassianSubdomainHint',
        placeholderKey: 'composio.connect.atlassianSubdomainPlaceholder',
        suffix: '.atlassian.net',
        validate: validateSubdomainLabel,
      },
    ],
    dynamics365: [
      {
        key: 'org_name',
        labelKey: 'composio.connect.dynamicsOrgNameLabel',
        hintKey: 'composio.connect.dynamicsOrgNameHint',
        placeholderKey: 'composio.connect.dynamicsOrgNamePlaceholder',
        suffix: '.crm.dynamics.com',
        validate: validateSubdomainLabel,
      },
    ],
  });

/** Return the required-field list for a toolkit slug (empty when none). */
export function getRequiredFieldsForToolkit(slug: string): readonly ToolkitRequiredField[] {
  return TOOLKIT_REQUIRED_FIELDS[slug] ?? [];
}

/**
 * Validate a values map against a toolkit's required-field definitions.
 * Returns a map of field-key → i18n error key. Empty map means all valid.
 */
export function validateRequiredFieldValues(
  fields: readonly ToolkitRequiredField[],
  values: Record<string, string>
): Record<string, TranslationKey> {
  const errors: Record<string, TranslationKey> = {};
  for (const field of fields) {
    const value = (values[field.key] ?? '').trim();
    if (!value) {
      errors[field.key] = 'composio.connect.requiredFieldEmpty';
      continue;
    }
    const customError = field.validate?.(value);
    if (customError) {
      errors[field.key] = customError;
    }
  }
  return errors;
}
