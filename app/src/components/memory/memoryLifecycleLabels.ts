/**
 * Labels for the memory lifecycle surface: brain source types, background job
 * kinds and job outcomes. Literal `t()` keys (not built from the value) so the
 * i18n scanner can see every one of them; an unknown value shows as-is.
 */
import type { JobOutcome } from '../../services/api/memoryApi';
import type { BadgeVariant } from '../ui';

type Translate = (key: string, fallback?: string) => string;

/** A brain source type's label: the well-known ones translated, any other id verbatim. */
export function brainSourceLabel(source: string, t: Translate): string {
  switch (source) {
    case 'files':
      return t('memoryPage.brain.source.files');
    case 'pdf':
      return t('memoryPage.brain.source.pdf');
    case 'markdown':
      return t('memoryPage.brain.source.markdown');
    case 'notion':
      return t('memoryPage.brain.source.notion');
    case 'github':
      return t('memoryPage.brain.source.github');
    case 'web':
      return t('memoryPage.brain.source.web');
    default:
      return source;
  }
}

/** A background job kind's label (`build_beliefs`, `ingest_brain`). */
export function jobKindLabel(kind: string, t: Translate): string {
  switch (kind) {
    case 'build_beliefs':
      return t('memoryPage.background.job.buildBeliefs');
    case 'ingest_brain':
      return t('memoryPage.background.job.ingestBrain');
    default:
      return kind;
  }
}

export function jobOutcomeLabel(outcome: JobOutcome, t: Translate): string {
  switch (outcome) {
    case 'done':
      return t('memoryPage.background.outcome.done');
    case 'started':
      return t('memoryPage.background.outcome.started');
    case 'scheduled':
      return t('memoryPage.background.outcome.scheduled');
    case 'skipped':
      return t('memoryPage.background.outcome.skipped');
    case 'failed':
      return t('memoryPage.background.outcome.failed');
    default:
      return String(outcome);
  }
}

export const JOB_OUTCOME_VARIANT: Record<JobOutcome, BadgeVariant> = {
  done: 'success',
  started: 'primary',
  scheduled: 'neutral',
  skipped: 'warning',
  failed: 'danger',
};
