import { getMcpRegistryErrorKind } from '../../../services/api/mcpRegistryErrors';

type Translate = (key: string, fallback?: string) => string;

export function mcpRegistryErrorMessage(error: unknown, t: Translate, fallbackKey: string): string {
  const kind = getMcpRegistryErrorKind(error);
  if (kind === 'not_found') return t('mcp.registry.error.notFound');
  if (kind === 'network') return t('mcp.registry.error.network');
  if (kind === 'unavailable') return t('mcp.registry.error.unavailable');
  return t(fallbackKey);
}
