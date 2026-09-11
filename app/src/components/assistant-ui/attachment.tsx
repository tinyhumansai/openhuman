'use client';

import { cn } from '@/components/assistant-ui/lib/utils';
import { TooltipIconButton } from '@/components/assistant-ui/tooltip-icon-button';
import { Avatar, AvatarFallback, AvatarImage } from '@/components/assistant-ui/ui/avatar';
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogTrigger,
} from '@/components/assistant-ui/ui/dialog';
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/assistant-ui/ui/tooltip';
import {
  AttachmentPrimitive,
  ComposerPrimitive,
  MessagePrimitive,
  useAui,
  useAuiState,
} from '@assistant-ui/react';
import { AlertCircleIcon, FileText, Loader2Icon, PlusIcon, XIcon } from 'lucide-react';
import { type FC, isValidElement, type PropsWithChildren, useEffect, useState } from 'react';
import { useShallow } from 'zustand/react/shallow';

const useFileSrc = (file: File | undefined) => {
  const [src, setSrc] = useState<string | undefined>(undefined);

  useEffect(() => {
    if (!file) {
      setSrc(undefined);
      return;
    }

    const objectUrl = URL.createObjectURL(file);
    setSrc(objectUrl);

    return () => {
      URL.revokeObjectURL(objectUrl);
    };
  }, [file]);

  return src;
};

const useAttachmentSrc = () => {
  const { file, src } = useAuiState(
    useShallow((s): { file?: File; src?: string } => {
      if (s.attachment.type !== 'image') return {};
      if (s.attachment.file) return { file: s.attachment.file };
      const src = s.attachment.content?.filter(c => c.type === 'image')[0]?.image;
      if (!src) return {};
      return { src };
    })
  );

  return useFileSrc(file) ?? src;
};

type AttachmentPreviewProps = { src: string };

const AttachmentPreview: FC<AttachmentPreviewProps> = ({ src }) => {
  const [isLoaded, setIsLoaded] = useState(false);
  return (
    <img
      src={src}
      alt="Attachment preview"
      className={cn(
        'block h-auto max-h-[80vh] w-auto max-w-full rounded-sm object-contain transition-opacity duration-300 motion-reduce:transition-none',
        isLoaded
          ? 'aui-attachment-preview-image-loaded opacity-100'
          : 'aui-attachment-preview-image-loading opacity-0'
      )}
      onLoad={() => setIsLoaded(true)}
    />
  );
};

const AttachmentPreviewDialog: FC<PropsWithChildren> = ({ children }) => {
  const src = useAttachmentSrc();

  if (!src) return children;

  return (
    <Dialog>
      <DialogTrigger
        nativeButton={false}
        className="aui-attachment-preview-trigger cursor-zoom-in"
        render={isValidElement(children) ? children : <button type="button" />}
      />
      <DialogContent className="aui-attachment-preview-dialog-content [&>button]:bg-foreground/60 [&>button]:hover:bg-foreground/80 [&_svg]:text-background p-2 sm:max-w-3xl [&>button]:rounded-full [&>button]:p-1 [&>button]:opacity-100 [&>button]:ring-0!">
        <DialogTitle className="aui-sr-only sr-only">Image Attachment Preview</DialogTitle>
        <div className="aui-attachment-preview bg-background relative mx-auto flex max-h-[80dvh] w-full items-center justify-center overflow-hidden rounded-sm">
          <AttachmentPreview src={src} />
        </div>
      </DialogContent>
    </Dialog>
  );
};

const AttachmentThumb: FC = () => {
  const src = useAttachmentSrc();

  return (
    <Avatar className="aui-attachment-tile-avatar h-full w-full rounded-none after:hidden">
      <AvatarImage
        src={src}
        alt="Attachment preview"
        className="aui-attachment-tile-image rounded-none object-cover"
      />
      <AvatarFallback>
        <FileText className="aui-attachment-tile-fallback-icon text-muted-foreground/80 size-6 stroke-[1.5]" />
      </AvatarFallback>
    </Avatar>
  );
};

const AttachmentUI: FC = () => {
  const aui = useAui();
  const isComposer = aui.attachment.source !== 'message';

  const isImage = useAuiState(s => s.attachment.type === 'image');
  const typeLabel = useAuiState(s => {
    const type = s.attachment.type;
    switch (type) {
      case 'image':
        return 'Image';
      case 'document':
        return 'Document';
      case 'file':
        return 'File';
      default:
        return type;
    }
  });

  const uploadState = useAuiState(s =>
    s.attachment.status.type === 'running'
      ? 'uploading'
      : s.attachment.status.type === 'incomplete' && s.attachment.status.reason === 'error'
        ? 'error'
        : undefined
  );
  const isUploading = uploadState === 'uploading';
  const isError = uploadState === 'error';

  const errorMessage = useAuiState(s =>
    s.attachment.status.type === 'incomplete' && s.attachment.status.reason === 'error'
      ? (s.attachment.status.message ?? 'Upload failed')
      : undefined
  );

  return (
    <TooltipProvider>
      <Tooltip>
        <AttachmentPrimitive.Root
          className={cn(
            'aui-attachment-root relative',
            isComposer && 'animate-in fade-in-0 zoom-in-95 duration-200 motion-reduce:animate-none',
            isImage && !isComposer && 'aui-attachment-root-message only:*:first:size-24'
          )}>
          <AttachmentPreviewDialog>
            <TooltipTrigger
              render={
                <div
                  className={cn(
                    'aui-attachment-tile bg-muted hover:after:bg-foreground/10 focus-visible:ring-ring/50 relative size-14 cursor-pointer overflow-hidden rounded-[calc(var(--composer-radius,1.5rem)-var(--composer-padding,8px))] transition-transform outline-none after:pointer-events-none after:absolute after:inset-0 after:rounded-[inherit] after:ring-1 after:ring-black/10 after:transition-colors after:ring-inset focus-visible:ring-1 active:scale-[0.96] motion-reduce:transition-none dark:after:ring-white/10',
                    isError && 'after:ring-destructive/60 dark:after:ring-destructive/60'
                  )}
                  role="button"
                  tabIndex={0}
                  aria-label={`${typeLabel} attachment${
                    isError ? ', upload failed' : isUploading ? ', uploading' : ''
                  }`}
                />
              }>
              <AttachmentThumb />
              {isUploading && (
                <div
                  aria-hidden="true"
                  className="aui-attachment-tile-uploading bg-background/60 animate-in fade-in-0 absolute inset-0 flex items-center justify-center backdrop-blur-[2px] motion-reduce:animate-none">
                  <Loader2Icon className="text-muted-foreground size-4 animate-spin" />
                </div>
              )}
              {isError && (
                <div
                  aria-hidden="true"
                  className="aui-attachment-tile-error bg-background/70 animate-in fade-in-0 absolute inset-0 flex items-center justify-center backdrop-blur-[2px] motion-reduce:animate-none">
                  <AlertCircleIcon className="text-destructive size-4" />
                </div>
              )}
            </TooltipTrigger>
          </AttachmentPreviewDialog>
          {isComposer && <AttachmentRemove />}
        </AttachmentPrimitive.Root>
        <TooltipContent side="top">
          <AttachmentPrimitive.Name />
          {errorMessage && <p className="aui-attachment-error-message">{errorMessage}</p>}
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
};

const AttachmentRemove: FC = () => {
  return (
    <AttachmentPrimitive.Remove
      render={
        <TooltipIconButton
          tooltip="Remove file"
          className="aui-attachment-tile-remove absolute end-1 top-1 size-5 rounded-full bg-black/50! text-white after:absolute after:-inset-1.5 hover:bg-black/70! hover:text-white! active:scale-[0.96] motion-reduce:transition-none"
          side="top"
        />
      }>
      <XIcon className="aui-attachment-remove-icon size-3 stroke-[2.5]" />
    </AttachmentPrimitive.Remove>
  );
};

export const UserMessageAttachments: FC = () => {
  return (
    <div className="aui-user-message-attachments-end col-span-full col-start-1 row-start-1 flex w-full flex-row justify-end gap-2">
      <MessagePrimitive.Attachments>{() => <AttachmentUI />}</MessagePrimitive.Attachments>
    </div>
  );
};

export const ComposerAttachments: FC = () => {
  return (
    <div className="aui-composer-attachments flex w-full flex-row items-center gap-2 overflow-x-auto empty:hidden">
      <ComposerPrimitive.Attachments>{() => <AttachmentUI />}</ComposerPrimitive.Attachments>
    </div>
  );
};

export const ComposerAddAttachment: FC = () => {
  return (
    <ComposerPrimitive.AddAttachment
      render={
        <TooltipIconButton
          tooltip="Add Attachment"
          side="bottom"
          variant="ghost"
          size="icon"
          className="aui-composer-add-attachment text-muted-foreground hover:text-foreground hover:bg-muted-foreground/15 dark:border-muted-foreground/15 dark:hover:bg-muted-foreground/30 size-7 rounded-full active:scale-[0.96] motion-reduce:transition-none"
          aria-label="Add Attachment"
        />
      }>
      <PlusIcon className="aui-attachment-add-icon size-4" />
    </ComposerPrimitive.AddAttachment>
  );
};
