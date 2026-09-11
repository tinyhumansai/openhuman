'use client';

import { cn } from '@/components/assistant-ui/lib/utils';
import type { ImageMessagePart, ImageMessagePartComponent } from '@assistant-ui/react';
import { cva, type VariantProps } from 'class-variance-authority';
import {
  CopyIcon,
  DownloadIcon,
  ImageIcon,
  ImageOffIcon,
  Loader2Icon,
  RefreshCwIcon,
  ShieldAlertIcon,
} from 'lucide-react';
import { memo, type PropsWithChildren, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

const extensionForMimeType = (mimeType?: string): string => {
  switch (mimeType) {
    case 'image/png':
      return 'png';
    case 'image/jpeg':
    case 'image/jpg':
      return 'jpg';
    case 'image/webp':
      return 'webp';
    case 'image/gif':
      return 'gif';
    case 'image/svg+xml':
      return 'svg';
    default:
      return 'png';
  }
};

const dataUriToBlob = (dataUri: string): Blob => {
  const commaIndex = dataUri.indexOf(',');
  const meta = commaIndex >= 0 ? dataUri.slice(0, commaIndex) : dataUri;
  const data = commaIndex >= 0 ? dataUri.slice(commaIndex + 1) : '';
  const mime = meta.match(/data:([^;]+)/i)?.[1]?.toLowerCase() ?? 'application/octet-stream';
  if (!/;base64/i.test(meta)) {
    const text = data.replace(/(?:%[0-9A-Fa-f]{2})+/g, seq => {
      try {
        return decodeURIComponent(seq);
      } catch {
        return seq;
      }
    });
    return new Blob([text], { type: mime });
  }
  const bytes = atob(data);
  const arr = new Uint8Array(bytes.length);
  for (let i = 0; i < bytes.length; i++) arr[i] = bytes.charCodeAt(i);
  return new Blob([arr], { type: mime });
};

const mimeFromImage = (image: string): string | undefined =>
  image.match(/^data:([^;,]+)/i)?.[1]?.toLowerCase();

const downloadImagePart = (part: Pick<ImageMessagePart, 'image' | 'filename'>): void => {
  if (typeof document === 'undefined') return;
  const ext = extensionForMimeType(mimeFromImage(part.image));
  const filename = part.filename ?? `image.${ext}`;
  const isDataUri = /^data:/i.test(part.image);
  const objectUrl = isDataUri ? URL.createObjectURL(dataUriToBlob(part.image)) : null;
  const href = objectUrl ?? part.image;
  const a = document.createElement('a');
  a.href = href;
  a.download = filename;
  a.rel = 'noopener';
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  if (objectUrl) setTimeout(() => URL.revokeObjectURL(objectUrl), 40_000);
};

const copyImagePart = async (part: Pick<ImageMessagePart, 'image'>): Promise<void> => {
  if (
    typeof navigator === 'undefined' ||
    !navigator.clipboard ||
    typeof ClipboardItem === 'undefined'
  ) {
    throw new Error('Clipboard API is not available in this environment.');
  }
  const blob = /^data:/i.test(part.image)
    ? dataUriToBlob(part.image)
    : await fetch(part.image).then(r => r.blob());
  const mime = mimeFromImage(part.image) ?? blob.type ?? 'image/png';
  await navigator.clipboard.write([new ClipboardItem({ [mime]: blob })]);
};

const imageVariants = cva('aui-image-root relative overflow-hidden rounded-lg', {
  variants: {
    variant: { outline: 'border-border border', ghost: '', muted: 'bg-muted/50' },
    size: { sm: 'max-w-64', default: 'max-w-96', lg: 'max-w-[512px]', full: 'w-full' },
  },
  defaultVariants: { variant: 'outline', size: 'default' },
});

export type ImageRootProps = React.ComponentProps<'div'> & VariantProps<typeof imageVariants>;

function ImageRoot({ className, variant, size, children, ...props }: ImageRootProps) {
  return (
    <div
      data-slot="image-root"
      data-variant={variant}
      data-size={size}
      className={cn(imageVariants({ variant, size, className }))}
      {...props}>
      {children}
    </div>
  );
}

type ImagePreviewProps = Omit<React.ComponentProps<'img'>, 'children'> & {
  containerClassName?: string;
};

function ImagePreview({
  className,
  containerClassName,
  onLoad,
  onError,
  alt = 'Image content',
  src,
  ...props
}: ImagePreviewProps) {
  const imgRef = useRef<HTMLImageElement>(null);
  const [loadedSrc, setLoadedSrc] = useState<string | undefined>(undefined);
  const [errorSrc, setErrorSrc] = useState<string | undefined>(undefined);

  const loaded = loadedSrc === src;
  const error = errorSrc === src;

  useEffect(() => {
    if (typeof src === 'string' && imgRef.current?.complete && imgRef.current.naturalWidth > 0) {
      setLoadedSrc(src);
    }
  }, [src]);

  return (
    <div data-slot="image-preview" className={cn('relative min-h-32', containerClassName)}>
      {!loaded && !error && (
        <div
          data-slot="image-preview-loading"
          className="bg-muted/50 absolute inset-0 flex items-center justify-center">
          <ImageIcon className="text-muted-foreground size-8 animate-pulse" />
        </div>
      )}
      {error ? (
        <div
          data-slot="image-preview-error"
          className="bg-muted/50 flex min-h-32 items-center justify-center p-4">
          <ImageOffIcon className="text-muted-foreground size-8" />
        </div>
      ) : (
        <img
          ref={imgRef}
          src={src}
          alt={alt}
          className={cn('block h-auto w-full object-contain', !loaded && 'invisible', className)}
          onLoad={e => {
            if (typeof src === 'string') setLoadedSrc(src);
            onLoad?.(e);
          }}
          onError={e => {
            if (typeof src === 'string') setErrorSrc(src);
            onError?.(e);
          }}
          {...props}
        />
      )}
    </div>
  );
}

function ImageFilename({ className, children, ...props }: React.ComponentProps<'span'>) {
  if (!children) return null;

  return (
    <span
      data-slot="image-filename"
      className={cn('text-muted-foreground block truncate px-2 py-1.5 text-xs', className)}
      {...props}>
      {children}
    </span>
  );
}

type ImageZoomProps = PropsWithChildren<{ src: string; alt?: string }>;

function ImageZoom({ src, alt = 'Image preview', children }: ImageZoomProps) {
  const [isMounted, setIsMounted] = useState(false);
  const [isOpen, setIsOpen] = useState(false);

  useEffect(() => {
    setIsMounted(true);
  }, []);

  const handleOpen = () => setIsOpen(true);
  const handleClose = () => setIsOpen(false);

  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setIsOpen(false);
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    const originalOverflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    return () => {
      document.body.style.overflow = originalOverflow;
    };
  }, [isOpen]);

  return (
    <>
      <div
        onClick={handleOpen}
        onKeyDown={e => e.key === 'Enter' && handleOpen()}
        role="button"
        tabIndex={0}
        className="aui-image-zoom-trigger cursor-zoom-in"
        aria-label="Click to zoom image">
        {children}
      </div>
      {isMounted &&
        isOpen &&
        createPortal(
          <div
            data-slot="image-zoom-overlay"
            role="button"
            tabIndex={0}
            className="aui-image-zoom-overlay fade-in animate-in fixed inset-0 z-50 flex items-center justify-center bg-black/80 duration-200"
            onClick={handleClose}
            onKeyDown={e => e.key === 'Enter' && handleClose()}
            aria-label="Close zoomed image">
            <img
              data-slot="image-zoom-content"
              src={src}
              alt={alt}
              className="aui-image-zoom-content fade-in zoom-in-95 animate-in max-h-[90vh] max-w-[90vw] cursor-zoom-out object-contain duration-200"
              onClick={e => {
                e.stopPropagation();
                handleClose();
              }}
            />
          </div>,
          document.body
        )}
    </>
  );
}

function ImageGenerating({ className }: { className?: string }) {
  return (
    <div
      data-slot="image-generating"
      className={cn('bg-muted/50 flex min-h-32 items-center justify-center p-4', className)}>
      <Loader2Icon className="text-muted-foreground size-8 animate-spin" />
      <span className="sr-only">Generating image…</span>
    </div>
  );
}

function ImageContentFilterError({ className, reason }: { className?: string; reason?: string }) {
  return (
    <div
      data-slot="image-content-filter-error"
      className={cn(
        'bg-muted/50 flex min-h-32 flex-col items-center justify-center gap-2 p-4 text-center',
        className
      )}>
      <ShieldAlertIcon className="text-muted-foreground size-8" />
      <p className="text-sm font-medium">Image could not be generated</p>
      {reason && <p className="text-muted-foreground text-xs">{reason}</p>}
    </div>
  );
}

export type ImageActionsProps = {
  part: ImageMessagePart;
  /**
   * Wire to your own generation call to show a regenerate button. The button
   * renders only when this is set and the part carries a `prompt`.
   */
  onRegenerate?: () => void | Promise<void>;
  className?: string;
};

function RegenerateButton({ onRegenerate }: { onRegenerate: () => void | Promise<void> }) {
  const [isRegenerating, setIsRegenerating] = useState(false);
  return (
    <button
      type="button"
      onClick={async () => {
        setIsRegenerating(true);
        try {
          await onRegenerate();
        } finally {
          setIsRegenerating(false);
        }
      }}
      disabled={isRegenerating}
      data-slot="image-regenerate"
      aria-label="Regenerate image"
      className="hover:bg-muted inline-flex size-7 items-center justify-center rounded disabled:opacity-50">
      <RefreshCwIcon className={cn('size-4', isRegenerating && 'animate-spin')} />
    </button>
  );
}

function ImageActions({ part, onRegenerate, className }: ImageActionsProps) {
  return (
    <div data-slot="image-actions" className={cn('flex items-center gap-1 p-1', className)}>
      <button
        type="button"
        onClick={() => downloadImagePart(part)}
        data-slot="image-download"
        aria-label="Download image"
        className="hover:bg-muted inline-flex size-7 items-center justify-center rounded">
        <DownloadIcon className="size-4" />
      </button>
      <button
        type="button"
        onClick={() => {
          copyImagePart(part).catch(() => {});
        }}
        data-slot="image-copy"
        aria-label="Copy image"
        className="hover:bg-muted inline-flex size-7 items-center justify-center rounded">
        <CopyIcon className="size-4" />
      </button>
      {onRegenerate && <RegenerateButton onRegenerate={onRegenerate} />}
    </div>
  );
}

const ImageImpl: ImageMessagePartComponent = props => {
  const { image, filename, status } = props;

  if (status?.type === 'running') {
    return (
      <ImageRoot>
        <ImageGenerating />
        <ImageFilename>{filename}</ImageFilename>
      </ImageRoot>
    );
  }

  if (status?.type === 'incomplete' && status.reason === 'content-filter') {
    return (
      <ImageRoot>
        <ImageContentFilterError reason="The provider blocked this image." />
      </ImageRoot>
    );
  }

  return (
    <ImageRoot>
      <ImageZoom src={image} alt={filename || 'Image content'}>
        <ImagePreview src={image} alt={filename || 'Image content'} />
      </ImageZoom>
      <ImageFilename>{filename}</ImageFilename>
    </ImageRoot>
  );
};

const Image = memo(ImageImpl) as unknown as ImageMessagePartComponent & {
  Root: typeof ImageRoot;
  Preview: typeof ImagePreview;
  Filename: typeof ImageFilename;
  Zoom: typeof ImageZoom;
  Actions: typeof ImageActions;
  Generating: typeof ImageGenerating;
  ContentFilterError: typeof ImageContentFilterError;
};

Image.displayName = 'Image';
Image.Root = ImageRoot;
Image.Preview = ImagePreview;
Image.Filename = ImageFilename;
Image.Zoom = ImageZoom;
Image.Actions = ImageActions;
Image.Generating = ImageGenerating;
Image.ContentFilterError = ImageContentFilterError;

export {
  Image,
  ImageRoot,
  ImagePreview,
  ImageFilename,
  ImageZoom,
  ImageActions,
  ImageGenerating,
  ImageContentFilterError,
  imageVariants,
};
