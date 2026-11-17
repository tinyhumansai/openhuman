import type { ToolCallMessagePartComponent } from '@assistant-ui/react';
import { FileTextIcon, PresentationIcon } from 'lucide-react';

import { ArtifactCard } from '../../../components/assistant-ui/elements/artifact-card';
import { Image } from '../../../components/assistant-ui/elements/image';
import { ImageGeneration } from '../../../components/assistant-ui/elements/image-generation';
import { Button } from '../../../components/ui';
import { useT } from '../../../lib/i18n/I18nContext';
import { useAuiThreadId } from '../../../providers/AssistantUiRuntimeProvider';
import { aiRegenerate } from '../../../services/chatService';
import { useAppSelector } from '../../../store/hooks';

/**
 * Result shape for `media_generate_image` / `media_generate_video`
 * (`crates/openhuman-core/src/media/generation/tools.rs`) per the
 * fe-brief's wire contract: an array of produced artifacts. `path` is a
 * local, core-served file (opened through the existing artifact
 * download/serve path — see `services/artifactDownloadService.ts`);
 * `source_url` is used directly when the artifact is
 * already externally hosted.
 */
interface MediaArtifact {
  type?: string;
  path?: string;
  source_url?: string;
  thumbnail_url?: string;
  artifact_id?: string;
}

function asMediaArtifacts(result: unknown): MediaArtifact[] | undefined {
  if (!result || typeof result !== 'object') return undefined;
  const artifacts = (result as { artifacts?: unknown }).artifacts;
  if (!Array.isArray(artifacts)) return undefined;
  return artifacts.filter((a): a is MediaArtifact => typeof a === 'object' && a !== null);
}

/**
 * `media_generate_image` / `media_generate_video`: the `elements-image-
 * generation` placeholder while the tool runs, then one element per produced
 * artifact — the vendored `elements/image.tsx` `Image` for `type: "image"`,
 * or a plain `<video>` (no vendored assistant-ui element covers video) for
 * `type: "video"`.
 *
 * `path` (a local, core-served artifact) is resolved through
 * `artifact_id` via the existing artifact download/reveal path rather than
 * dereferenced directly — an artifact's on-disk location is not a stable
 * URL a plain `<img>`/`<video>` can load without the core's static file
 * route, and that route is what `services/artifactDownloadService.ts`
 * already knows how to reach. Until the wire contract confirms the served
 * URL shape, the local-path case falls back to `thumbnail_url`/`source_url`
 * when present and otherwise skips the artifact rather than guessing a path.
 */
export const MediaGenerationCall: ToolCallMessagePartComponent = ({
  toolName,
  args,
  result,
  status,
}) => {
  const prompt =
    typeof (args as { prompt?: unknown })?.prompt === 'string'
      ? (args as { prompt: string }).prompt
      : '';
  const running = status?.type === 'running';
  const artifacts = asMediaArtifacts(result) ?? [];

  if (running || artifacts.length === 0) {
    return <ImageGeneration prompt={prompt} generating={running} />;
  }

  return (
    <div className="flex flex-wrap gap-2" data-testid="assistant-ui-media-generation-result">
      {artifacts.map((artifact, index) => {
        const key = artifact.artifact_id ?? `${artifact.path ?? 'artifact'}-${index}`;
        const isVideo = artifact.type === 'video' || toolName === 'media_generate_video';
        if (isVideo) {
          const src = artifact.source_url;
          if (!src) return null;
          return (
            <video
              key={key}
              data-testid="assistant-ui-media-generation-video"
              src={src}
              poster={artifact.thumbnail_url}
              controls
              className="max-h-72 max-w-full rounded-lg"
            />
          );
        }
        const src = artifact.source_url ?? artifact.thumbnail_url;
        if (!src) return null;
        return <Image key={key} type="image" image={src} status={{ type: 'complete' }} />;
      })}
    </div>
  );
};

const DOCUMENT_TOOL_ICONS: Record<string, typeof FileTextIcon> = {
  generate_document: FileTextIcon,
  generate_presentation: PresentationIcon,
};

/**
 * `generate_document` / `generate_presentation`
 * (`crates/openhuman-core/src/tools/impl/{document,presentation}/mod.rs`):
 * the `elements-artifact-card` element, generating (with a rough word count
 * from the call's args) while the tool runs, settling to the produced
 * artifact's title once it returns.
 */
export const DocumentArtifactCall: ToolCallMessagePartComponent = ({
  toolCallId,
  toolName,
  args,
  result,
  status,
}) => {
  const { t } = useT();
  const threadId = useAuiThreadId();
  // A failed `artifact_failed` snapshot for THIS call, when the producing
  // tool reported one and the core sent `tool_call_id` on the event —
  // `Conversations.tsx` filters an artifact carrying `toolCallId` OUT of the
  // header's live-artifact deck precisely so it renders here instead.
  const failedArtifact = useAppSelector(state => {
    const list = threadId ? state.chatRuntime.artifactsByThread[threadId] : undefined;
    return list?.find(a => a.toolCallId === toolCallId && a.status === 'failed');
  });
  const running = status?.type === 'running';
  const Icon = DOCUMENT_TOOL_ICONS[toolName] ?? FileTextIcon;
  const kindTitle =
    toolName === 'generate_presentation'
      ? t('conversations.tools.presentation.title', 'Presentation')
      : t('conversations.tools.document.title', 'Document');

  const title =
    (typeof (result as { title?: unknown })?.title === 'string'
      ? (result as { title: string }).title
      : undefined) ??
    (typeof (args as { title?: unknown })?.title === 'string'
      ? (args as { title: string }).title
      : undefined) ??
    kindTitle;

  // No live token count from the core mid-generation; approximate from the
  // args payload so the shimmering "N words" line has something to show
  // rather than staying frozen at zero.
  const words = running
    ? JSON.stringify(args ?? '')
        .split(/\s+/)
        .filter(Boolean).length
    : 0;

  const meta =
    typeof (result as { path?: unknown })?.path === 'string'
      ? (result as { path: string }).path
      : kindTitle;

  if (failedArtifact) {
    return (
      <div className="flex flex-col items-start gap-1.5">
        <ArtifactCard
          title={title}
          meta={t('chat.artifact.failed')}
          generating={false}
          icon={Icon}
        />
        {threadId ? (
          <Button
            variant="secondary"
            size="xs"
            analyticsId="chat-artifact-retry-inline"
            onClick={() => {
              void aiRegenerate(failedArtifact.artifactId, threadId).catch(err => {
                console.warn('[artifact] regenerate failed:', err);
              });
            }}>
            {t('chat.artifact.retry')}
          </Button>
        ) : null}
      </div>
    );
  }

  return <ArtifactCard title={title} meta={meta} generating={running} words={words} icon={Icon} />;
};
