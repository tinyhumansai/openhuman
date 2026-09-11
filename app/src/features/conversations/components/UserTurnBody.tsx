import { useEffect, useState } from 'react';

import type { ThreadMessage } from '../../../types/thread';
import { formatRelativeTime } from '../utils/format';
import { BubbleMarkdown } from './AgentMessageBubble';

// Matches only well-formed base64 image data URIs — guards against an
// `<img src>` XSS vector if a persisted message ever carried a crafted
// value in `attachmentDataUris`/legacy `[IMAGE:...]` markers.
export const SAFE_IMAGE_DATA_URI_RE =
  /^data:(image\/(?:png|jpe?g|gif|webp|bmp));base64,([a-z0-9+/=\s]+)$/i;
const EMPTY_IMAGE_SRC = 'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==';

function imageDataUriToObjectUrl(src: string): string | null {
  const match = SAFE_IMAGE_DATA_URI_RE.exec(src);
  if (!match) return null;
  try {
    const mime = match[1];
    const binary = atob(match[2].replace(/\s/g, ''));
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i += 1) {
      bytes[i] = binary.charCodeAt(i);
    }
    return URL.createObjectURL(new Blob([bytes], { type: mime }));
  } catch {
    return null;
  }
}

function AttachmentImage({ dataUri }: { dataUri: string }) {
  const [objectUrl, setObjectUrl] = useState<string | null>(null);

  useEffect(() => {
    const nextUrl = imageDataUriToObjectUrl(dataUri);
    setObjectUrl(nextUrl);
    return () => {
      if (nextUrl) URL.revokeObjectURL(nextUrl);
    };
  }, [dataUri]);

  return (
    <img
      src={objectUrl ?? EMPTY_IMAGE_SRC}
      alt=""
      className="max-w-[200px] max-h-[200px] rounded-2xl object-cover"
    />
  );
}

export interface UserTurnBodyProps {
  msg: ThreadMessage;
  displayText: string;
  fallbackDataUris: string[];
  showTime: boolean;
}

/**
 * A user turn: its attachments (images, videos, files) above its bubble.
 *
 * Split out of `TranscriptRow` because the attachment rendering — three kinds
 * of chip, plus the object-URL lifecycle behind `AttachmentImage` — is the bulk
 * of the row's code and none of it is reached on the agent side.
 */
export function UserTurnBody({ msg, displayText, fallbackDataUris, showTime }: UserTurnBodyProps) {
  const dataUris = (
    Array.isArray(msg.extraMetadata?.attachmentDataUris)
      ? (msg.extraMetadata.attachmentDataUris as string[])
      : fallbackDataUris
  ).filter(src => SAFE_IMAGE_DATA_URI_RE.test(src));
  // Document attachments carry no image data-URI (only images do); surface them
  // as filename chips from the persisted attachmentKinds/attachmentNames metadata.
  const kinds = Array.isArray(msg.extraMetadata?.attachmentKinds)
    ? (msg.extraMetadata.attachmentKinds as string[])
    : [];
  const names = Array.isArray(msg.extraMetadata?.attachmentNames)
    ? (msg.extraMetadata.attachmentNames as string[])
    : [];
  const fileNames = kinds
    .map((k, i) => (k === 'file' ? names[i] : null))
    .filter((n): n is string => Boolean(n));
  const posters = Array.isArray(msg.extraMetadata?.attachmentPosters)
    ? (msg.extraMetadata.attachmentPosters as (string | null)[])
    : [];
  const videoItems = kinds
    .map((k, i) => (k === 'video' ? { name: names[i] ?? '', poster: posters[i] ?? null } : null))
    .filter((v): v is { name: string; poster: string | null } => Boolean(v));

  return (
    <div className="flex flex-col items-end gap-1">
      {dataUris.length > 0 && (
        <div className="flex flex-wrap gap-1.5 justify-end">
          {dataUris.map((uri, i) => (
            <AttachmentImage key={i} dataUri={uri} />
          ))}
        </div>
      )}
      {videoItems.length > 0 && (
        <div className="flex flex-wrap gap-1.5 justify-end">
          {videoItems.map((video, i) => (
            <div
              key={i}
              className="relative flex items-center gap-2 rounded-lg border border-line bg-surface-muted px-2.5 py-1.5 text-xs text-content-secondary max-w-[220px]">
              {video.poster ? (
                <div className="relative w-10 h-10 shrink-0">
                  <img src={video.poster} alt="" className="w-10 h-10 rounded object-cover" />
                  <span className="absolute inset-0 flex items-center justify-center">
                    <svg
                      className="w-4 h-4 text-content-inverted drop-shadow-sm"
                      fill="currentColor"
                      viewBox="0 0 24 24">
                      <path d="M8 5v14l11-7z" />
                    </svg>
                  </span>
                </div>
              ) : (
                <svg
                  className="w-4 h-4 shrink-0 text-content-muted"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={1.8}
                    d="M15 10l4.553-2.276A1 1 0 0121 8.618v6.764a1 1 0 01-1.447.894L15 14M5 6h8a2 2 0 012 2v8a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2z"
                  />
                </svg>
              )}
              <span className="truncate font-medium">{video.name}</span>
            </div>
          ))}
        </div>
      )}
      {fileNames.length > 0 && (
        <div className="flex flex-wrap gap-1.5 justify-end">
          {fileNames.map((name, i) => (
            <div
              key={i}
              className="flex items-center gap-2 rounded-lg border border-line bg-surface-muted px-2.5 py-1.5 text-xs text-content-secondary max-w-[220px]">
              <svg
                className="w-4 h-4 shrink-0 text-content-muted"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={1.8}
                  d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z"
                />
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={1.8}
                  d="M14 2v6h6"
                />
              </svg>
              <span className="truncate font-medium">{name}</span>
            </div>
          ))}
        </div>
      )}
      {(displayText || showTime) && (
        <div className="rounded-2xl px-4 py-2.5 bg-primary-500 text-content-inverted rounded-br-md wrap-break-word wrap-anywhere overflow-hidden">
          {displayText && <BubbleMarkdown content={displayText} tone="user" />}
          {showTime && (
            <p className={`${displayText ? 'mt-1' : ''} text-[10px] text-content-inverted/60`}>
              {formatRelativeTime(msg.createdAt)}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
