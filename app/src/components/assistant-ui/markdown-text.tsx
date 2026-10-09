'use client';

import { cn } from '@/components/assistant-ui/lib/utils';
import { TooltipIconButton } from '@/components/assistant-ui/tooltip-icon-button';
import { type AssistantState, useAuiState, useMessagePartText } from '@assistant-ui/react';
import {
  type CodeHeaderProps,
  MarkdownTextPrimitive,
  unstable_memoizeMarkdownComponents as memoizeMarkdownComponents,
  useIsMarkdownCodeBlock,
} from '@assistant-ui/react-markdown';
import '@assistant-ui/react-markdown/styles/dot.css';
import { CheckIcon, CopyIcon } from 'lucide-react';
import {
  type ComponentPropsWithoutRef,
  createContext,
  type FC,
  isValidElement,
  memo,
  useContext,
  useMemo,
  useState,
} from 'react';
import rehypeHighlight from 'rehype-highlight';
import rehypeKatex from 'rehype-katex';
import remarkGfm from 'remark-gfm';
import remarkMath from 'remark-math';

import { ExternalSchemeLink } from '../../features/conversations/tools/mcpUi/LinkActions';
import { classifyHref, transformChatUrl } from '../../features/conversations/utils/format';
import { hasLatexContent, normalizeLatexDelimiters } from '../../utils/latex';
import { extractLanguage, extractTextContent } from '../markdown/CodeBlock';
import { CitationMarker, type CitationSource } from './elements/inline-citation';

/**
 * This message's `source` parts (`SourceGroupSlot` in `thread.tsx` reads the
 * same parts for the disclosure under the answer), reduced to the vendored
 * `inline-citation` element's `CitationSource` shape and made available to
 * the `a` node override below — `defaultComponents` is a module-level,
 * memoized map (`memoizeMarkdownComponents`), so a per-message value has to
 * reach its components through context rather than a closure.
 */
const CitationSourcesContext = createContext<readonly CitationSource[]>([]);
const EMPTY_MESSAGE_PARTS: AssistantState['message']['parts'] = [];

function sourcePartsToCitations(parts: AssistantState['message']['parts']): CitationSource[] {
  return parts.flatMap((part): CitationSource[] => {
    if (part.type !== 'source') return [];
    if (part.sourceType === 'url') {
      let domain = part.url;
      try {
        domain = new URL(part.url).hostname.replace(/^www\./, '');
      } catch {
        // Keep the raw value; a malformed URL still names its own citation.
      }
      return [{ domain, title: part.title ?? domain, snippet: part.url }];
    }
    return [{ domain: 'memory', title: part.title ?? 'memory', snippet: part.title ?? '' }];
  });
}

/**
 * `[n]` / `[^n]` in the model's own text, for `n` within the message's
 * source count, become a real markdown link to a `#citation-n` fragment —
 * a relative ref, so react-markdown's default `urlTransform` allowlist
 * (which blanks any URL scheme it does not recognize, e.g. a `citation:`
 * one) leaves it alone. The `a` node override below recognizes that
 * fragment shape and swaps in `CitationMarker` instead of an anchor.
 * Everything else (an ordinary bracketed aside, a footnote number past
 * the source list) is left alone.
 */
function linkifyCitationMarkers(text: string, sourceCount: number): string {
  if (sourceCount === 0) return text;
  return text.replace(/\[\^?(\d+)\]/g, (match, digits: string) => {
    const n = Number.parseInt(digits, 10);
    return n >= 1 && n <= sourceCount ? `[${digits}](#citation-${digits})` : match;
  });
}

/**
 * Plugin sets, matched to `AgentMessageBubble`'s so the two markdown surfaces
 * cannot disagree about the same message. Module-level constants because a new
 * array identity on every render makes `react-markdown` re-parse the whole
 * document — on a streaming answer that is once per token.
 *
 * `rehypeHighlight` must precede `rehypeKatex` so code blocks inside a math
 * environment are not processed twice (same ordering, same reason, as
 * `AgentMessageBubble.tsx`).
 */
const GFM_REMARK_PLUGINS = [remarkGfm];
const MATH_REMARK_PLUGINS = [remarkGfm, remarkMath];
const HIGHLIGHT_REHYPE_PLUGINS = [rehypeHighlight];
const MATH_REHYPE_PLUGINS = [rehypeHighlight, rehypeKatex];

/** Chat links minus workspace references, which this surface cannot open. */
function transformAssistantUrl(url: string): string {
  return classifyHref(url) === 'workspace' ? '' : transformChatUrl(url);
}

const MarkdownTextImpl = () => {
  // Math is GATED, not always-on, and the gate is `hasLatexContent` rather than
  // "contains a $". `remark-math` would otherwise read "$10 vs $20" as an inline
  // formula and eat the prose between them; the signature requires a real LaTeX
  // signal (`\frac`, `\begin`, `\[`, `\(`, `$$`). Same call the legacy
  // surface makes, so a message renders identically on both.
  //
  // Read the raw part text, not the smoothed text `MarkdownTextPrimitive`
  // renders: the gate must not flip mid-reveal.
  const { text } = useMessagePartText();
  const hasMath = hasLatexContent(text);
  // Some callers (e.g. a bare `TextMessagePartProvider` in tests, or a tool
  // result rendered through `MarkdownText` outside a full message scope)
  // provide a message-PART scope with no message-level `state.message` — the
  // proxy throws reading it. No sources to linkify is the correct fallback,
  // not a crash.
  //
  // The selector returns the raw `parts` array rather than a derived
  // `CitationSource[]` on purpose: `useAuiState` runs this through
  // `useSyncExternalStore`, which requires a snapshot-stable result — a fresh
  // `.flatMap()` array on every call sends it into a render loop ("Maximum
  // update depth exceeded"). Deriving `sources` in a `useMemo` below, keyed
  // on this array's own identity, keeps the selector pure and the derived
  // value stable across renders that don't change the underlying parts.
  const parts = useAuiState(state => {
    try {
      return state.message.parts;
    } catch {
      return EMPTY_MESSAGE_PARTS;
    }
  });
  const sources = useMemo(() => sourcePartsToCitations(parts), [parts]);

  const preprocess = (input: string): string => {
    const withCitations = linkifyCitationMarkers(input, sources.length);
    return hasMath ? normalizeLatexDelimiters(withCitations) : withCitations;
  };

  return (
    <CitationSourcesContext.Provider value={sources}>
      <MarkdownTextPrimitive
        remarkPlugins={hasMath ? MATH_REMARK_PLUGINS : GFM_REMARK_PLUGINS}
        rehypePlugins={hasMath ? MATH_REHYPE_PLUGINS : HIGHLIGHT_REHYPE_PLUGINS}
        // `\[ … \]` / `\( … \)` are what models actually emit; `remark-math`
        // only understands `$ … $`. Citation linkification always runs
        // (a no-op when the message has no sources); LaTeX normalization is
        // gated as before. Runs before the smooth reveal, so the text is
        // normalised once rather than per frame.
        preprocess={preprocess}
        className="aui-md"
        urlTransform={transformAssistantUrl}
        components={defaultComponents}
        defer
      />
    </CitationSourcesContext.Provider>
  );
};

export const MarkdownText = memo(MarkdownTextImpl);

const CodeHeader: FC<CodeHeaderProps> = ({ language, code }) => {
  const { isCopied, copyToClipboard } = useCopyToClipboard();
  const onCopy = () => {
    if (!code || isCopied) return;
    copyToClipboard(code);
  };

  return (
    <div className="aui-code-header-root border-border/50 bg-muted/50 mt-3 flex items-center justify-between rounded-t-xl border border-b-0 px-3.5 py-1.5 text-xs">
      <span className="aui-code-header-language text-muted-foreground font-medium lowercase">
        {language}
      </span>
      <TooltipIconButton tooltip="Copy" onClick={onCopy}>
        {!isCopied && <CopyIcon className="animate-in zoom-in-75 fade-in duration-150" />}
        {isCopied && <CheckIcon className="animate-in zoom-in-50 fade-in duration-200 ease-out" />}
      </TooltipIconButton>
    </div>
  );
};

const useCopyToClipboard = ({ copiedDuration = 3000 }: { copiedDuration?: number } = {}) => {
  const [isCopied, setIsCopied] = useState<boolean>(false);

  const copyToClipboard = (value: string) => {
    if (!value || typeof navigator === 'undefined' || !navigator.clipboard) {
      return;
    }

    navigator.clipboard.writeText(value).then(
      () => {
        setIsCopied(true);
        setTimeout(() => setIsCopied(false), copiedDuration);
      },
      () => {}
    );
  };

  return { isCopied, copyToClipboard };
};

/** The slice of a hast `<code>` element this file reads. */
type HastElement = { properties?: { className?: string | string[] } };

/**
 * The fenced-code block: our header bar, then the code body.
 *
 * The header is rendered HERE rather than through the kit's `CodeHeader` slot,
 * and that is load-bearing. `rehypeHighlight` replaces the `<code>` element's
 * single string child with `<span class="hljs-*">` nodes, and the kit branches
 * on exactly that: a string child takes the `DefaultCodeBlock` path (header +
 * body), anything else takes `DefaultCodeBlockContent` (body only). So wiring
 * the highlighter through the `CodeHeader` slot silently drops the language
 * label and the copy button from every block that has a language — i.e. every
 * block a reader cares about — while leaving them on untagged ones. Verified
 * both ways before this was written.
 *
 * `Pre` is called on both paths, so owning the header here gives one code path
 * instead of two and the highlighted and unhighlighted cases render alike.
 */
const CodeBlockPre: FC<ComponentPropsWithoutRef<'pre'>> = ({ className, children, ...props }) => {
  // The child is the kit's wrapped `<code>`. Its React props carry only `node`
  // and `children` — the `className` is merged in downstream, inside the
  // wrapper — so the `language-*` class has to be read off the hast node, where
  // `className` is an array of tokens rather than a string.
  const codeProps = isValidElement<{ node?: HastElement; children?: unknown }>(children)
    ? children.props
    : undefined;
  const hastClassName = codeProps?.node?.properties?.className;

  return (
    <>
      <CodeHeader
        language={
          extractLanguage(Array.isArray(hastClassName) ? hastClassName.join(' ') : hastClassName) ??
          undefined
        }
        code={extractTextContent(codeProps?.children)}
      />
      <pre
        className={cn(
          'aui-md-pre border-border/50 bg-muted/30 overflow-x-auto rounded-t-none rounded-b-xl border border-t-0 p-3.5 text-[13px] leading-relaxed',
          className
        )}
        {...props}>
        {children}
      </pre>
    </>
  );
};

const defaultComponents = memoizeMarkdownComponents({
  h1: ({ className, ...props }) => (
    <h1
      className={cn(
        'aui-md-h1 mt-5 mb-2 scroll-m-20 text-xl font-semibold first:mt-0 last:mb-0',
        className
      )}
      {...props}
    />
  ),
  h2: ({ className, ...props }) => (
    <h2
      className={cn(
        'aui-md-h2 mt-5 mb-2 scroll-m-20 text-lg font-semibold first:mt-0 last:mb-0',
        className
      )}
      {...props}
    />
  ),
  h3: ({ className, ...props }) => (
    <h3
      className={cn(
        'aui-md-h3 mt-4 mb-1.5 scroll-m-20 text-base font-semibold first:mt-0 last:mb-0',
        className
      )}
      {...props}
    />
  ),
  h4: ({ className, ...props }) => (
    <h4
      className={cn(
        'aui-md-h4 mt-3.5 mb-1 scroll-m-20 text-base font-medium first:mt-0 last:mb-0',
        className
      )}
      {...props}
    />
  ),
  h5: ({ className, ...props }) => (
    <h5
      className={cn('aui-md-h5 mt-3 mb-1 text-sm font-semibold first:mt-0 last:mb-0', className)}
      {...props}
    />
  ),
  h6: ({ className, ...props }) => (
    <h6
      className={cn('aui-md-h6 mt-3 mb-1 text-sm font-medium first:mt-0 last:mb-0', className)}
      {...props}
    />
  ),
  p: ({ className, ...props }) => (
    <p className={cn('aui-md-p my-3 leading-relaxed first:mt-0 last:mb-0', className)} {...props} />
  ),
  a: function MarkdownLink({ className, href, children, ...props }) {
    const sources = useContext(CitationSourcesContext);
    const citationMatch = href?.match(/^#citation-(\d+)$/);
    const citationIndex = citationMatch ? Number.parseInt(citationMatch[1], 10) - 1 : -1;
    const source = citationIndex >= 0 ? sources[citationIndex] : undefined;
    if (source) return <CitationMarker index={citationIndex} source={source} />;
    if (href && classifyHref(href) === 'handoff') {
      return <ExternalSchemeLink href={href}>{children}</ExternalSchemeLink>;
    }
    return (
      <a
        className={cn('aui-md-a text-primary hover:text-primary/80 no-underline', className)}
        href={href}
        {...props}>
        {children}
      </a>
    );
  },
  blockquote: ({ className, ...props }) => (
    <blockquote
      className={cn(
        'aui-md-blockquote border-muted-foreground/30 text-muted-foreground my-3 border-s-2 ps-4',
        className
      )}
      {...props}
    />
  ),
  ul: ({ className, ...props }) => (
    <ul
      className={cn(
        'aui-md-ul marker:text-muted-foreground my-3 ms-5 list-disc [&>li]:mt-1',
        className
      )}
      {...props}
    />
  ),
  ol: ({ className, ...props }) => (
    <ol
      className={cn(
        'aui-md-ol marker:text-muted-foreground my-3 ms-5 list-decimal [&>li]:mt-1',
        className
      )}
      {...props}
    />
  ),
  hr: ({ className, ...props }) => (
    <hr className={cn('aui-md-hr border-muted-foreground/20 my-3', className)} {...props} />
  ),
  table: ({ className, ...props }) => (
    <table
      className={cn(
        'aui-md-table my-3 w-full border-separate border-spacing-0 overflow-y-auto',
        className
      )}
      {...props}
    />
  ),
  th: ({ className, ...props }) => (
    <th
      className={cn(
        'aui-md-th bg-muted px-3 py-1.5 text-start font-medium first:rounded-ss-lg last:rounded-se-lg [[align=center]]:text-center [[align=right]]:text-right',
        className
      )}
      {...props}
    />
  ),
  td: ({ className, ...props }) => (
    <td
      className={cn(
        'aui-md-td border-muted-foreground/20 border-s border-b px-3 py-1.5 text-start last:border-e [[align=center]]:text-center [[align=right]]:text-right',
        className
      )}
      {...props}
    />
  ),
  tr: ({ className, ...props }) => (
    <tr
      className={cn(
        'aui-md-tr m-0 border-b p-0 first:border-t [&:last-child>td:first-child]:rounded-es-lg [&:last-child>td:last-child]:rounded-ee-lg',
        className
      )}
      {...props}
    />
  ),
  li: ({ className, ...props }) => (
    <li className={cn('aui-md-li leading-relaxed', className)} {...props} />
  ),
  strong: ({ className, ...props }) => (
    <strong className={cn('aui-md-strong font-semibold', className)} {...props} />
  ),
  sup: ({ className, ...props }) => (
    <sup className={cn('aui-md-sup [&>a]:text-xs [&>a]:no-underline', className)} {...props} />
  ),
  pre: CodeBlockPre,
  code: function Code({ className, ...props }) {
    const isCodeBlock = useIsMarkdownCodeBlock();
    return (
      <code
        className={cn(
          !isCodeBlock &&
            'aui-md-inline-code bg-muted rounded-md px-1.5 py-0.5 font-mono text-[0.85em]',
          className
        )}
        {...props}
      />
    );
  },
});
