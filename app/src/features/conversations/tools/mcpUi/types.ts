/**
 * The `mcp_ui` presentation the core attaches to a tool call that offered a
 * widget or a link (`crates/openhuman-core/src/mcp/ui/types.rs`).
 */
export type McpUiFlavor = 'mcp_apps' | 'apps_sdk' | 'host_inline';

export type McpUiLinkKind = 'external' | 'handoff';

export interface McpUiLink {
  url: string;
  kind: McpUiLinkKind;
}

export interface McpUiPresentation {
  kind: 'mcp_ui';
  flavor: McpUiFlavor;
  server_id?: string;
  tool: string;
  resource_uri?: string;
  inline_id?: string;
  title?: string;
  tool_input?: unknown;
  structured_content?: unknown;
  result_meta?: unknown;
  links: McpUiLink[];
}

/** A widget document as `openhuman.mcp_ui_resource_read` returns it. */
export interface McpUiResource {
  html: string;
  mime_type: string;
  csp: { connect_domains: string[]; resource_domains: string[] };
  permissions?: unknown;
  prefers_border: boolean;
}

export interface McpUiToolCallResult {
  requires_confirmation: boolean;
  read_only: boolean;
  result?: unknown;
  is_error?: boolean;
}

const FLAVORS: readonly McpUiFlavor[] = ['mcp_apps', 'apps_sdk', 'host_inline'];

const IMAGE_PATH = /\.(?:jpe?g|png|gif|webp|avif|svg|bmp|ico|heic|heif|tiff?)$/i;

/** Whether an `http(s)` URL points at an image asset rather than a page. */
export function isImageUrl(url: string): boolean {
  let path: string;
  try {
    path = new URL(url).pathname.toLowerCase();
  } catch {
    return false;
  }
  return path.includes('/image/upload/') || IMAGE_PATH.test(path);
}

function isLink(value: unknown): value is McpUiLink {
  if (!value || typeof value !== 'object') return false;
  const link = value as Record<string, unknown>;
  if (typeof link.url !== 'string') return false;
  if (link.kind === 'handoff') return true;
  return link.kind === 'external' && !isImageUrl(link.url);
}

/** Narrows a tool call's `structured` payload to a presentation. */
export function readMcpUiPresentation(structured: unknown): McpUiPresentation | null {
  if (!structured || typeof structured !== 'object' || Array.isArray(structured)) return null;
  const value = structured as Record<string, unknown>;
  if (value.kind !== 'mcp_ui') return null;
  if (typeof value.tool !== 'string') return null;
  const flavor = FLAVORS.includes(value.flavor as McpUiFlavor)
    ? (value.flavor as McpUiFlavor)
    : 'mcp_apps';
  const optionalString = (key: string) =>
    typeof value[key] === 'string' && (value[key] as string).length > 0
      ? (value[key] as string)
      : undefined;
  return {
    kind: 'mcp_ui',
    flavor,
    tool: value.tool,
    server_id: optionalString('server_id'),
    resource_uri: optionalString('resource_uri'),
    inline_id: optionalString('inline_id'),
    title: optionalString('title'),
    tool_input: value.tool_input,
    structured_content: value.structured_content,
    result_meta: value.result_meta,
    links: Array.isArray(value.links) ? value.links.filter(isLink) : [],
  };
}

/** Whether the presentation has a widget document to load. */
export function hasWidget(presentation: McpUiPresentation): boolean {
  if (presentation.inline_id) return true;
  return Boolean(presentation.resource_uri && presentation.server_id);
}
