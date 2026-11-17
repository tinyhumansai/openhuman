/**
 * Labels and target hints per Documents source kind. Literal `t()` keys (not
 * built from the kind) so the i18n scanner can see every one of them.
 */
import type { DocumentSourceKind, SourceStatus } from '../../services/api/memoryApi';
import type { BadgeVariant } from '../ui';

type Translate = (key: string, fallback?: string) => string;

export function sourceKindLabel(kind: DocumentSourceKind, t: Translate): string {
  switch (kind) {
    case 'folder':
      return t('memoryPage.sourceKind.folder');
    case 'file':
      return t('memoryPage.sourceKind.file');
    case 'link':
      return t('memoryPage.sourceKind.link');
    case 'github':
      return t('memoryPage.sourceKind.github');
    case 'rss':
      return t('memoryPage.sourceKind.rss');
    default:
      return String(kind);
  }
}

/** What the target field expects for a kind: a path, a URL, `owner/repo`, … */
export function sourceTargetHint(kind: DocumentSourceKind, t: Translate): string {
  switch (kind) {
    case 'folder':
      return t('memoryPage.sourceTarget.folder');
    case 'file':
      return t('memoryPage.sourceTarget.file');
    case 'link':
      return t('memoryPage.sourceTarget.link');
    case 'github':
      return t('memoryPage.sourceTarget.github');
    case 'rss':
      return t('memoryPage.sourceTarget.rss');
    default:
      return '';
  }
}

export const SOURCE_STATUS_VARIANT: Record<SourceStatus, BadgeVariant> = {
  idle: 'neutral',
  syncing: 'primary',
  error: 'danger',
};

export function sourceStatusLabel(status: SourceStatus, t: Translate): string {
  switch (status) {
    case 'syncing':
      return t('memoryPage.sourceStatus.syncing');
    case 'error':
      return t('memoryPage.sourceStatus.error');
    default:
      return t('memoryPage.sourceStatus.idle');
  }
}
