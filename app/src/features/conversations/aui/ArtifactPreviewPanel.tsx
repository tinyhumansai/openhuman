'use client';

import { Check, Clipboard, Code2, Download, Eye, FileBox, FileText, Share2, X } from 'lucide-react';
import { type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import Markdown from 'react-markdown';
import remarkGfm from 'remark-gfm';

import ChipTabs from '../../../components/layout/ChipTabs';
import {
  hasWindowDragBar,
  WINDOW_DRAG_BAR_HEIGHT,
} from '../../../components/layout/shell/WindowDragBar';
import { Button } from '../../../components/ui';
import { useT } from '../../../lib/i18n/I18nContext';
import { saveArtifactViaDialog } from '../../../services/artifactDownloadService';
import { callCoreRpc } from '../../../services/coreRpcClient';
import {
  activateArtifactTab,
  type ArtifactSnapshot,
  closeArtifactPanel,
  closeArtifactTab,
} from '../../../store/chatRuntimeSlice';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import { openUrl } from '../../../utils/openUrl';
import { isAllowedExternalHref } from '../utils/format';

// ─── Types ──────────────────────────────────────────────────────

interface ArtifactPreviewPanelProps {
  /** No props: the panel reads openArtifactIds/activeArtifactId from Redux. */
}

interface ArtifactBodyResponse {
  kind: string;
  title: string;
  version: number;
  body: string;
  is_text: boolean;
  /** `body` is Markdown converted from a document file (DOCX, PDF, ...). */
  converted?: boolean;
}

// ─── Constants ──────────────────────────────────────────────────

const MIN_WIDTH = 360;
const MAX_WIDTH = 1200;
const DEFAULT_WIDTH = 520;
/** Width change per arrow key on the resize handle. */
const RESIZE_STEP = 24;

// ─── Markdown links ─────────────────────────────────────────────

/**
 * A link in a Markdown preview: a click never navigates the app's window. An
 * external link (`http(s)`, `mailto`) opens in the system browser, as chat
 * links do; anything else does nothing.
 */
function PreviewLink({ href, children }: { href?: string; children?: ReactNode }) {
  return (
    <a
      href={href}
      onClick={e => {
        e.preventDefault();
        if (!href || !isAllowedExternalHref(href)) return;
        void openUrl(href).catch(err => {
          console.warn('[ArtifactPreviewPanel] could not open link:', err);
        });
      }}
      className="cursor-pointer underline wrap-break-word">
      {children}
    </a>
  );
}

const PREVIEW_MARKDOWN_COMPONENTS = { a: PreviewLink };

// ─── Main panel ─────────────────────────────────────────────────

export function ArtifactPreviewPanel(_props: ArtifactPreviewPanelProps) {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const [panelWidth, setPanelWidth] = useState(DEFAULT_WIDTH);
  const dragRef = useRef(false);
  const startXRef = useRef(0);
  const startWidthRef = useRef(0);

  // All artifacts for the current thread — we show them as tabs
  const artifactsByThread = useAppSelector(s => s.chatRuntime.artifactsByThread);
  const allArtifacts = useMemo(() => {
    const list: ArtifactSnapshot[] = [];
    for (const arr of Object.values(artifactsByThread)) {
      for (const a of arr) {
        if (a.status === 'ready') list.push(a);
      }
    }
    return list.sort((a, b) => (b.updatedAt ?? 0) - (a.updatedAt ?? 0));
  }, [artifactsByThread]);

  // Active tab and open-tab list come from Redux — single source of truth.
  const openArtifactIds = useAppSelector(s => s.chatRuntime.openArtifactIds);
  const activeId = useAppSelector(s => s.chatRuntime.activeArtifactId);
  const activeArtifact = useMemo(
    () => allArtifacts.find(a => a.artifactId === activeId),
    [allArtifacts, activeId]
  );

  // ── Resize ──
  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      dragRef.current = true;
      startXRef.current = e.clientX;
      startWidthRef.current = panelWidth;
      document.body.style.cursor = 'ew-resize';
      document.body.style.userSelect = 'none';
    },
    [panelWidth]
  );

  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      if (!dragRef.current) return;
      const delta = startXRef.current - e.clientX;
      setPanelWidth(Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, startWidthRef.current + delta)));
    };
    const onUp = () => {
      if (!dragRef.current) return;
      dragRef.current = false;
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
    return () => {
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
    };
  }, []);

  // The panel is on the right, so moving the handle left widens it.
  const handleResizeKey = useCallback((e: React.KeyboardEvent) => {
    const delta = e.key === 'ArrowLeft' ? RESIZE_STEP : e.key === 'ArrowRight' ? -RESIZE_STEP : 0;
    if (!delta) return;
    e.preventDefault();
    setPanelWidth(width => Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, width + delta)));
  }, []);

  // Tabs = the artifacts currently open in the panel, in open order.
  const visibleTabs = useMemo(
    () =>
      openArtifactIds
        .map(id => allArtifacts.find(a => a.artifactId === id))
        .filter((a): a is ArtifactSnapshot => a !== undefined),
    [openArtifactIds, allArtifacts]
  );
  const handleCloseTab = useCallback(
    (id: string) => {
      dispatch(closeArtifactTab(id));
    },
    [dispatch]
  );

  const showTabs = visibleTabs.length > 1;

  return (
    <div
      className="relative flex flex-col border-l bg-background shadow-xl h-full"
      style={{
        width: panelWidth,
        minWidth: MIN_WIDTH,
        maxWidth: MAX_WIDTH,
        // The panel header is at the top of the window, under the window-drag
        // band, which would otherwise swallow clicks on the tabs and the X.
        paddingTop: hasWindowDragBar() ? WINDOW_DRAG_BAR_HEIGHT : undefined,
      }}>
      {/* Resize handle: drag it, or focus it and use the arrow keys. */}
      <div
        role="separator"
        aria-orientation="vertical"
        aria-label={t('chat.artifact.panel.resize')}
        aria-valuemin={MIN_WIDTH}
        aria-valuemax={MAX_WIDTH}
        aria-valuenow={panelWidth}
        tabIndex={0}
        onMouseDown={handleMouseDown}
        onKeyDown={handleResizeKey}
        className="absolute left-0 top-0 bottom-0 w-1.5 cursor-ew-resize outline-none hover:bg-primary/20 focus-visible:bg-primary/30 active:bg-primary/30 z-20"
      />

      {/* Header */}
      <div className="flex items-center justify-between border-b shrink-0">
        <div className="flex items-center gap-1 min-w-0 flex-1 overflow-x-auto">
          {showTabs ? (
            <div
              role="tablist"
              aria-label={t('chat.artifact.tabs.aria')}
              className="flex items-center gap-1 min-w-0">
              {visibleTabs.map(a => {
                const active = a.artifactId === activeId;
                return (
                  <div
                    key={a.artifactId}
                    className={`group flex items-center gap-1 border-b-2 px-2 py-2.5 transition-colors shrink-0 min-w-0 ${
                      active ? 'border-primary' : 'border-transparent'
                    }`}>
                    <Button
                      role="tab"
                      aria-selected={active}
                      variant="tertiary"
                      size="xs"
                      onClick={() => dispatch(activateArtifactTab(a.artifactId))}
                      leadingIcon={<FileText className="h-3.5 w-3.5 shrink-0" />}
                      className={`h-auto min-w-0 px-0 hover:bg-transparent ${
                        active ? 'text-foreground' : 'text-muted-foreground hover:text-foreground'
                      }`}>
                      <span className="truncate max-w-[110px]">{a.title}</span>
                    </Button>
                    <Button
                      iconOnly
                      variant="tertiary"
                      size="xs"
                      aria-label={t('chat.artifact.tab.close').replace('{title}', a.title)}
                      onClick={e => {
                        e.stopPropagation();
                        handleCloseTab(a.artifactId);
                      }}
                      className="ml-0.5 text-muted-foreground/50 hover:text-foreground shrink-0">
                      <X className="h-3 w-3" />
                    </Button>
                  </div>
                );
              })}
            </div>
          ) : (
            <div className="flex items-center gap-2 px-4 py-2.5 min-w-0">
              <FileText className="h-4 w-4 shrink-0 text-muted-foreground" />
              <span className="text-sm font-medium truncate">
                {activeArtifact?.title || t('chat.artifact.preview')}
              </span>
            </div>
          )}
        </div>
        <Button
          iconOnly
          variant="tertiary"
          size="sm"
          aria-label={t('chat.artifact.panel.close')}
          onClick={() => dispatch(closeArtifactPanel())}
          className="mr-2 shrink-0 text-muted-foreground hover:text-foreground">
          <X className="h-4 w-4" />
        </Button>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-hidden">
        {activeArtifact ? (
          <ArtifactContentPanel key={activeArtifact.artifactId} artifact={activeArtifact} />
        ) : (
          <div className="flex items-center justify-center h-full p-4 text-sm text-muted-foreground">
            {t('chat.artifact.notFound')}
          </div>
        )}
      </div>
    </div>
  );
}

// ─── Content panel (fetches and renders) ────────────────────────

function ArtifactContentPanel({ artifact }: { artifact: ArtifactSnapshot }) {
  // Mounted per artifact (keyed by id), so switching starts from a clean state.
  const [bodyResult, setBodyResult] = useState<ArtifactBodyResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!artifact.artifactId) {
      setLoading(false);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);

    const fetchBody = async () => {
      try {
        // Read from the core the app talks to (local or remote): it opens the
        // file and converts documents where the artifact lives.
        const result = await callCoreRpc<ArtifactBodyResponse>({
          method: 'openhuman.ai_read_artifact_body',
          params: { artifact_id: artifact.artifactId },
        });
        if (!cancelled) setBodyResult(result);
      } catch (err) {
        if (!cancelled) setError(String(err));
      }
      if (!cancelled) setLoading(false);
    };

    fetchBody();
    return () => {
      cancelled = true;
    };
  }, [artifact.artifactId]);

  // A new version saved while the panel is open (the agent revised it, or an
  // autosave landed): read it again, without the loading state.
  const seenUpdate = useRef(artifact.updatedAt);
  useEffect(() => {
    if (seenUpdate.current === artifact.updatedAt) return;
    seenUpdate.current = artifact.updatedAt;
    if (!artifact.artifactId || artifact.status !== 'ready') return;
    let cancelled = false;
    void callCoreRpc<ArtifactBodyResponse>({
      method: 'openhuman.ai_read_artifact_body',
      params: { artifact_id: artifact.artifactId },
    })
      .then(result => {
        if (!cancelled) setBodyResult(result);
      })
      .catch(err => console.warn('[ArtifactPreviewPanel] refresh failed:', err));
    return () => {
      cancelled = true;
    };
  }, [artifact.artifactId, artifact.status, artifact.updatedAt]);

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full p-8">
        <div className="h-5 w-5 animate-spin rounded-full border-2 border-primary border-t-transparent" />
      </div>
    );
  }

  const kind = bodyResult?.kind || artifact.kind;
  const body = bodyResult?.body || '';
  const version = bodyResult?.version || 1;
  const isText = bodyResult?.is_text ?? false;
  const converted = bodyResult?.converted ?? false;

  // File kind whose body is actual readable text → editable viewer.
  // Truly binary files (body stays a path) → FileViewer (metadata + download).
  if (kind === 'file' || kind === 'image') {
    if (isText) {
      return (
        <EditableTextViewer
          artifact={artifact}
          body={body}
          version={version}
          kind={kind}
          error={error}
          converted={converted}
        />
      );
    }
    return <FileViewer artifact={artifact} body={body} version={version} error={error} />;
  }

  // Text-based kinds → EditableTextViewer (preview + code editor + save)
  return (
    <EditableTextViewer
      artifact={artifact}
      body={body}
      version={version}
      kind={kind}
      error={error}
    />
  );
}

// ─── File viewer (binary / file-reference kinds) ────────────────

function FileViewer({
  artifact,
  body,
  version,
  error,
}: {
  artifact: ArtifactSnapshot;
  body: string;
  version: number;
  error: string | null;
}) {
  const { t } = useT();
  const [downloadError, setDownloadError] = useState<string | null>(null);
  const filePath = body || artifact.path || '';
  const fileName = filePath.split('/').pop() || filePath;
  // Share is hidden until the backend serves viewable files inline (links download today); see useArtifactShare.
  // const sharing = useArtifactShare(artifact.artifactId);

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center justify-between border-b px-3 py-1.5 shrink-0 bg-muted/10">
        <div className="flex items-center gap-2">
          <FileBox className="h-4 w-4 text-muted-foreground" />
          <span className="text-xs font-medium text-muted-foreground">
            {t(`chat.artifact.kind.${artifact.kind}`, artifact.kind)}
          </span>
          <span className="text-[11px] text-muted-foreground/60 border-l pl-2">v{version}</span>
        </div>
        {/* <ShareButton state={sharing.state} onShare={() => void sharing.share()} /> */}
      </div>
      {/* <ShareStatus state={sharing.state} onCopy={sharing.copyAgain} /> */}

      <div className="flex-1 overflow-auto p-6">
        <div className="flex flex-col items-center justify-center gap-4 text-center">
          <FileBox className="h-12 w-12 text-muted-foreground/40" />
          <div>
            <p className="text-sm font-medium break-all">{fileName}</p>
            <p className="text-xs text-muted-foreground mt-1 break-all">{filePath}</p>
            <p className="text-xs text-muted-foreground mt-2">{t('chat.artifact.noPreview')}</p>
            {error && <p className="text-xs text-destructive mt-2">{error}</p>}
            {downloadError && <p className="text-xs text-destructive mt-2">{downloadError}</p>}
          </div>
          {filePath && (
            <Button
              size="sm"
              leadingIcon={<Download className="h-3.5 w-3.5" />}
              onClick={() => void saveArtifactAs(artifact, t).then(setDownloadError)}>
              {t('chat.artifact.download')}
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}

// ─── Download and autosave ──────────────────────────────────────

/** Versioned kinds whose record holds the content, so an edit is a version. */
const EDITABLE_KINDS = new Set(['text', 'markdown', 'html']);

/** Times a failed autosave is retried on its own before waiting for the user. */
export const AUTOSAVE_MAX_RETRIES = 3;

/** Wait after the last keystroke before saving the edit as a new version. */
export const AUTOSAVE_DELAY_MS = 5_000;

/** The artifact's file name: the one it has in the files folder. */
/**
 * Let the user pick where to save the artifact's file (the OS save dialog),
 * through the download service. Resolves to a localized error message, or
 * `null` when saved or cancelled.
 */
async function saveArtifactAs(
  artifact: ArtifactSnapshot,
  t: (key: string) => string
): Promise<string | null> {
  const extension = fileExtension(artifact.path);
  const outcome = await saveArtifactViaDialog(artifact.artifactId, artifact.title, extension);
  if (outcome.ok || outcome.code === 'CANCELLED') return null;
  console.warn('[ArtifactPreviewPanel] download failed:', outcome.code, outcome.error);
  return t(`chat.files.error.${(outcome.code ?? 'download_failed').toLowerCase()}`);
}

/** The extension of the file a path names (`'md'` for `.../notes.md`), or `''`. */
export function fileExtension(path: string | undefined): string {
  const name = path?.split(/[\\/]/).pop() ?? '';
  return /\.([^.]+)$/.exec(name)?.[1] ?? '';
}

type AutosaveState = 'saved' | 'pending' | 'saving' | 'error';

/**
 * Save edits as new operator versions, AUTOSAVE_DELAY_MS after the last
 * change. A pending edit is saved at once when the artifact changes, the
 * panel closes, or a download needs the newest file. The core appends nothing
 * when the text matches the newest version.
 */
export function useArtifactAutosave(
  artifactId: string,
  body: string,
  initialVersion: number,
  enabled: boolean
) {
  const [status, setStatus] = useState<AutosaveState>('saved');
  const [version, setVersion] = useState(initialVersion);
  const pending = useRef<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  // The retry after a failed save calls the current `save`.
  const saveRef = useRef<() => Promise<void>>(async () => {});
  // Retries since the last successful save or edit, so a save that keeps
  // failing (also after the panel closed) stops instead of retrying forever.
  const retries = useRef(0);

  useEffect(() => {
    setVersion(initialVersion);
    // A new body read in while an edit is pending does not make that edit
    // saved.
    if (pending.current === null) setStatus('saved');
  }, [artifactId, body, initialVersion]);

  const save = useCallback(async () => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
    const next = pending.current;
    pending.current = null;
    if (next === null) return;
    setStatus('saving');
    try {
      const result = await callCoreRpc<{ version: number }>({
        method: 'openhuman.ai_save_artifact_version',
        params: { artifact_id: artifactId, body: next },
      });
      retries.current = 0;
      if (typeof result?.version === 'number') setVersion(result.version);
      setStatus(pending.current === null ? 'saved' : 'pending');
    } catch (err) {
      console.error('[ArtifactPreviewPanel] autosave failed:', err);
      setStatus('error');
      // Keep the edit and try again later, unless a newer edit replaced it
      // meanwhile (that one is saved instead).
      if (pending.current === null) {
        pending.current = next;
        // The edit stays pending; past the cap only a new edit or a flush
        // (download, closing the panel) tries again.
        if (retries.current >= AUTOSAVE_MAX_RETRIES) return;
        retries.current += 1;
        if (timer.current) clearTimeout(timer.current);
        timer.current = setTimeout(() => void saveRef.current(), AUTOSAVE_DELAY_MS);
      }
    }
  }, [artifactId]);
  useEffect(() => {
    saveRef.current = save;
  }, [save]);

  const schedule = useCallback(
    (next: string) => {
      if (!enabled) return;
      retries.current = 0;
      pending.current = next;
      setStatus('pending');
      if (timer.current) clearTimeout(timer.current);
      timer.current = setTimeout(() => void save(), AUTOSAVE_DELAY_MS);
    },
    [enabled, save]
  );

  // Save a pending edit when switching artifacts or closing the panel.
  useEffect(() => () => void save(), [save]);

  // Drop a held edit and its retry: a newer version saved elsewhere replaced
  // it.
  const discard = useCallback(() => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
    pending.current = null;
    retries.current = 0;
    setStatus('saved');
  }, []);

  return useMemo(
    () => ({ status, version, schedule, flush: save, discard }),
    [status, version, schedule, save, discard]
  );
}

function AutosaveStatus({ status }: { status: AutosaveState }) {
  const { t } = useT();
  const label = t(`chat.artifact.autosave.${status}`);
  return (
    <span
      className={`text-[11px] border-l pl-2 ${
        status === 'error' ? 'text-destructive' : 'text-muted-foreground/60'
      }`}>
      {label}
    </span>
  );
}

// ─── Share ──────────────────────────────────────────────────────

interface ShareResult {
  url: string;
  version: number;
  public: boolean;
  expires_at_millis?: number | null;
  reused: boolean;
}

type ShareState =
  | { status: 'idle' }
  | { status: 'sharing' }
  | { status: 'shared'; url: string; version: number; expiresAt: number | null; copied: boolean }
  | { status: 'error'; message: string };

/**
 * Share the artifact's newest version through cloud file storage
 * (`ai.share_artifact`, which uploads it and returns a link — reused while it
 * still works) and copy the link. `beforeShare` runs first, e.g. to save a
 * pending edit so the newest text is what is shared.
 */
export function useArtifactShare(artifactId: string, beforeShare?: () => Promise<void>) {
  const [state, setState] = useState<ShareState>({ status: 'idle' });

  useEffect(() => {
    setState({ status: 'idle' });
  }, [artifactId]);

  const copy = useCallback(async (url: string) => {
    try {
      await navigator.clipboard.writeText(url);
      return true;
    } catch {
      return false;
    }
  }, []);

  const share = useCallback(async () => {
    setState({ status: 'sharing' });
    try {
      await beforeShare?.();
      const result = await callCoreRpc<ShareResult>({
        method: 'openhuman.ai_share_artifact',
        params: { artifact_id: artifactId },
      });
      const copied = await copy(result.url);
      setState({
        status: 'shared',
        url: result.url,
        version: result.version,
        expiresAt: result.expires_at_millis ?? null,
        copied,
      });
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      console.error('[ArtifactPreviewPanel] share failed:', err);
      setState({ status: 'error', message });
    }
  }, [artifactId, beforeShare, copy]);

  const copyAgain = useCallback(async () => {
    if (state.status !== 'shared') return false;
    const copied = await copy(state.url);
    setState({ ...state, copied });
    return copied;
  }, [copy, state]);

  return { state, share, copyAgain };
}

export function ShareButton({ state, onShare }: { state: ShareState; onShare: () => void }) {
  const { t } = useT();
  return (
    <Button
      iconOnly
      variant="tertiary"
      size="xs"
      onClick={onShare}
      disabled={state.status === 'sharing'}
      aria-label={t('chat.artifact.share')}
      title={t('chat.artifact.share')}
      className="text-muted-foreground hover:text-foreground">
      {state.status === 'sharing' ? (
        <div className="h-3.5 w-3.5 animate-spin rounded-full border-2 border-primary border-t-transparent" />
      ) : (
        <Share2 className="h-3.5 w-3.5" />
      )}
    </Button>
  );
}

/** How long the Copy button says "Copied" after a click. */
const COPIED_FEEDBACK_MS = 2000;

export function ShareStatus({
  state,
  onCopy,
}: {
  state: ShareState;
  onCopy: () => Promise<boolean>;
}) {
  const { t } = useT();
  // The button's own feedback: only for a click on it, not for the copy the
  // share already made (the status text reports that one).
  const [justCopied, setJustCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    []
  );
  const handleCopy = useCallback(async () => {
    const copied = await onCopy();
    if (!copied) return;
    setJustCopied(true);
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => setJustCopied(false), COPIED_FEEDBACK_MS);
  }, [onCopy]);

  if (state.status === 'error') {
    return (
      <div
        data-testid="artifact-share-status"
        className="border-b bg-destructive/10 px-3 py-1.5 text-[11px] text-destructive">
        {t('chat.artifact.share.failed').replace('{reason}', state.message)}
      </div>
    );
  }
  if (state.status !== 'shared') return null;
  const expires = state.expiresAt
    ? ` · ${t('chat.artifact.share.expires').replace(
        '{date}',
        new Date(state.expiresAt).toLocaleDateString()
      )}`
    : '';
  return (
    <div
      data-testid="artifact-share-status"
      className="flex items-center gap-2 border-b bg-muted/20 px-3 py-1.5 text-[11px] text-muted-foreground">
      <span className="shrink-0">
        {state.copied ? t('chat.artifact.share.copied') : t('chat.artifact.share.ready')} · v
        {state.version}
        {expires}
      </span>
      <span className="min-w-0 flex-1 truncate text-foreground/70" title={state.url}>
        {state.url}
      </span>
      <Button
        variant="secondary"
        size="xs"
        onClick={() => void handleCopy()}
        title={t('chat.artifact.share.copyLink')}
        data-testid="artifact-share-copy"
        leadingIcon={
          justCopied ? (
            <Check className="h-3 w-3 text-green-500" />
          ) : (
            <Clipboard className="h-3 w-3" />
          )
        }
        className="shrink-0 text-[11px]">
        {justCopied ? t('chat.artifact.copied') : t('chat.artifact.share.copy')}
      </Button>
    </div>
  );
}

// ─── Editable text viewer (markdown / text / html) ──────────────

function EditableTextViewer({
  artifact,
  body,
  version,
  kind,
  error,
  converted = false,
}: {
  artifact: ArtifactSnapshot;
  body: string;
  version: number;
  kind: string;
  error: string | null;
  /** The body is Markdown converted from a document file: render it as such. */
  converted?: boolean;
}) {
  const [tab, setTab] = useState<'preview' | 'code'>('preview');
  const [editedBody, setEditedBody] = useState(body);
  const [copied, setCopied] = useState(false);
  const { t } = useT();
  const [downloadError, setDownloadError] = useState<string | null>(null);
  // Only kinds whose record holds the content itself can take a new version;
  // a `file` record holds a path, and its text here is read off that file.
  // A body that failed to load is not the artifact's content: editing it
  // would save the empty text over it.
  const editable = EDITABLE_KINDS.has(kind) && !error;
  const autosave = useArtifactAutosave(artifact.artifactId, body, version, editable);

  // Take a new body (another artifact, or a version saved elsewhere) unless
  // an edit here is still on its way: that edit is saved as the next version.
  // An edit whose save failed is not on its way: the new version replaces it,
  // and its retry is dropped so it cannot overwrite that version later.
  const statusRef = useRef(autosave.status);
  statusRef.current = autosave.status;
  const discardRef = useRef(autosave.discard);
  discardRef.current = autosave.discard;
  useEffect(() => {
    const status = statusRef.current;
    if (status === 'pending' || status === 'saving') return;
    if (status === 'error') discardRef.current();
    setEditedBody(body);
  }, [body]);

  const onEdit = useCallback(
    (next: string) => {
      setEditedBody(next);
      autosave.schedule(next);
    },
    [autosave]
  );

  const handleCopy = useCallback(() => {
    void navigator.clipboard.writeText(editedBody);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }, [editedBody]);

  const handleDownload = useCallback(async () => {
    // The file is the newest saved version: save a pending edit first.
    await autosave.flush();
    setDownloadError(await saveArtifactAs(artifact, t));
  }, [artifact, autosave, t]);

  // Share the newest saved version: save a pending edit first.
  // Share is hidden until the backend serves viewable files inline (links download today); see useArtifactShare.
  // const sharing = useArtifactShare(artifact.artifactId, autosave.flush);

  const isHtml = kind === 'html';
  const isMarkdown = kind === 'markdown' || converted;

  return (
    <div className="flex flex-col h-full">
      {/* Toolbar */}
      <div className="flex items-center justify-between border-b px-3 py-1.5 shrink-0 bg-muted/10">
        <div className="flex items-center gap-2">
          <span className="text-xs font-medium text-muted-foreground">
            {t(`chat.artifact.kind.${kind}`, kind)}
          </span>
          <span className="text-[11px] text-muted-foreground/60 border-l pl-2">
            v{autosave.version}
          </span>
          {editable && <AutosaveStatus status={autosave.status} />}
        </div>
        <div className="flex items-center gap-0.5">
          <ChipTabs
            compact
            ariaLabel={t('chat.artifact.view.aria')}
            className="mr-2 flex gap-1"
            value={tab}
            onChange={setTab}
            items={[
              {
                id: 'preview',
                label: (
                  <span className="inline-flex items-center gap-1">
                    <Eye className="h-3 w-3" />
                    {t('chat.artifact.view.preview')}
                  </span>
                ),
              },
              {
                id: 'code',
                label: (
                  <span className="inline-flex items-center gap-1">
                    <Code2 className="h-3 w-3" />
                    {t('chat.artifact.view.code')}
                  </span>
                ),
              },
            ]}
          />

          <Button
            iconOnly
            variant="tertiary"
            size="xs"
            onClick={handleCopy}
            aria-label={copied ? t('chat.artifact.copied') : t('chat.artifact.copy')}
            title={t('chat.artifact.copy')}
            className="text-muted-foreground hover:text-foreground">
            {copied ? (
              <Check className="h-3.5 w-3.5 text-green-500" />
            ) : (
              <Clipboard className="h-3.5 w-3.5" />
            )}
          </Button>
          {/* <ShareButton state={sharing.state} onShare={() => void sharing.share()} /> */}
          <Button
            iconOnly
            variant="tertiary"
            size="xs"
            onClick={() => void handleDownload()}
            aria-label={t('chat.artifact.download')}
            title={t('chat.artifact.download')}
            className="text-muted-foreground hover:text-foreground">
            <Download className="h-3.5 w-3.5" />
          </Button>
        </div>
      </div>

      {/* <ShareStatus state={sharing.state} onCopy={sharing.copyAgain} /> */}

      {/* Error banner */}
      {downloadError && (
        <div className="border-b bg-destructive/10 px-3 py-1.5 text-[11px] text-destructive">
          {downloadError}
        </div>
      )}
      {error && (
        <div className="border-b bg-destructive/10 px-3 py-1.5 text-[11px] text-destructive">
          {error}
        </div>
      )}

      {/* Content area */}
      {tab === 'preview' ? (
        isHtml ? (
          <iframe
            srcDoc={editedBody}
            className="w-full flex-1 border-0"
            // Scripts run, but in an opaque origin: the agent's HTML cannot
            // reach the app window, its storage or the Tauri bridge.
            sandbox="allow-scripts"
            title={t('chat.artifact.htmlPreview')}
          />
        ) : isMarkdown ? (
          // Markdown renders as formatted content (headings, lists, tables,
          // emphasis) just like HTML renders in the iframe above — not as a
          // raw source dump.
          <div className="flex-1 overflow-auto">
            <div className="p-4 prose prose-sm dark:prose-invert max-w-none">
              <Markdown remarkPlugins={[remarkGfm]} components={PREVIEW_MARKDOWN_COMPONENTS}>
                {editedBody}
              </Markdown>
            </div>
          </div>
        ) : (
          <div className="flex-1 overflow-auto">
            <pre className="p-4 text-sm font-mono leading-relaxed whitespace-pre-wrap break-words">
              {editedBody}
            </pre>
          </div>
        )
      ) : (
        <textarea
          value={editedBody}
          onChange={e => onEdit(e.target.value)}
          readOnly={!editable}
          aria-label={t('chat.artifact.editor')}
          className="flex-1 w-full resize-none border-0 bg-background p-4 text-sm font-mono leading-relaxed outline-none focus:ring-0"
          spellCheck={false}
        />
      )}
    </div>
  );
}

export default ArtifactPreviewPanel;
