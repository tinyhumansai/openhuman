'use client';

/**
 * Composer-header "live artifact deck" adapter over the vendored
 * `elements/artifact-card.tsx` `ArtifactCard`, replacing the deleted
 * `components/chat/ArtifactCard.tsx`.
 *
 * Only artifacts with NO owning tool call (`ArtifactSnapshot.toolCallId`
 * absent) reach this deck — `Conversations.tsx`'s `liveArtifactDeck` filters
 * those out; an artifact with a `toolCallId` renders inline through its own
 * tool-call card instead (`MediaAndDocumentCalls.tsx`).
 *
 * The vendored card has no built-in failed/retry state (it only knows
 * "writing" vs. "settled"), so the failed case renders the settled meta line
 * with the failure reason and an explicit Retry button underneath, wired to
 * the same `aiRegenerate` re-dispatch the legacy card used.
 */
import { FileTextIcon, ImageIcon, PresentationIcon, VideoIcon } from 'lucide-react';
import type { ElementType } from 'react';

import { ArtifactCard } from '../../../components/assistant-ui/elements/artifact-card';
import { Button } from '../../../components/ui';
import { formatFileSize } from '../../../lib/attachments';
import { useT } from '../../../lib/i18n/I18nContext';
import type { ArtifactSnapshot } from '../../../store/chatRuntimeSlice';

const KIND_ICONS: Record<ArtifactSnapshot['kind'], ElementType> = {
  presentation: PresentationIcon,
  document: FileTextIcon,
  image: ImageIcon,
  video: VideoIcon,
  other: FileTextIcon,
};

export interface ArtifactCardAdapterProps {
  artifact: ArtifactSnapshot;
  /** When provided, render a Retry affordance on the `failed` state. */
  onRetry?: (artifactId: string) => void;
  /** When provided, clicking a ready artifact opens the preview panel. */
  onOpenPreview?: (artifactId: string) => void;
}

/** The core's prefix for a version saved but not shown (`production::SAVED_NOT_SHOWN`). */
export const SAVED_NOT_SHOWN = 'saved-not-shown:';

export function ArtifactCardAdapter({
  artifact,
  onRetry,
  onOpenPreview,
}: ArtifactCardAdapterProps) {
  const { t } = useT();
  const generating = artifact.status === 'in_progress';
  const isReady = artifact.status === 'ready';
  // The version was saved, but its file in the files folder was not updated:
  // the artifact opens (the preview reads the saved version) and there is
  // nothing to retry.
  const savedNotShown =
    artifact.status === 'failed' && (artifact.error ?? '').startsWith(SAVED_NOT_SHOWN);
  const meta =
    isReady && artifact.sizeBytes != null
      ? `${t('chat.artifact.ready')} · ${formatFileSize(artifact.sizeBytes)}`
      : savedNotShown
        ? t('chat.artifact.savedNotShown')
        : artifact.status === 'failed'
          ? t('chat.artifact.failed')
          : '';
  const canOpen = isReady || savedNotShown;

  // The card's own button is the one control that opens the preview.
  return (
    <div className="flex flex-col items-start gap-1.5" data-testid="artifact-card-adapter">
      <ArtifactCard
        title={artifact.title}
        meta={meta}
        generating={generating}
        words={0}
        writingLabel={t('conversations.tools.working')}
        icon={KIND_ICONS[artifact.kind]}
        onOpen={canOpen && onOpenPreview ? () => onOpenPreview(artifact.artifactId) : undefined}
      />
      {artifact.status === 'failed' && !savedNotShown && onRetry ? (
        <Button
          variant="secondary"
          size="xs"
          analyticsId="chat-artifact-retry"
          onClick={() => onRetry(artifact.artifactId)}>
          {t('chat.artifact.retry')}
        </Button>
      ) : null}
    </div>
  );
}

export default ArtifactCardAdapter;
