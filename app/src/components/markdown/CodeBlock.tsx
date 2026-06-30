/**
 * CodeBlock.tsx
 *
 * Factory that produces a custom `pre` component for react-markdown's
 * `components` prop, closing over the bubble `tone`. Renders:
 *   - A header bar with a language label (left) and a copy-to-clipboard
 *     button (right).
 *   - The code body with syntax-highlighting classes applied by
 *     rehype-highlight (upstream in the rehype plugin chain).
 *
 * Usage:
 *   const codePre = useMemo(() => createCodeBlockPre(tone), [tone]);
 *   <Markdown components={{ pre: codePre }}>...</Markdown>
 */
import { type ComponentPropsWithoutRef, type ReactElement, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';

// ── Helpers ─────────────────────────────────────────────────────────────────

/**
 * Extract the `language-xxx` class from a code element's className string
 * and return only the language part (e.g. "typescript"), or null if not
 * present. rehype-highlight adds this class when a fenced block is tagged.
 */
function extractLanguage(className?: string): string | null {
  if (!className) return null;
  const match = /\blanguage-(\S+)/.exec(className);
  return match ? match[1] : null;
}

/**
 * Make a language identifier human-readable for display in the header badge.
 * Short tags stay uppercase; longer ones get title-cased.
 */
function formatLanguageLabel(lang: string): string {
  if (lang.length <= 4) return lang.toUpperCase();
  return lang.charAt(0).toUpperCase() + lang.slice(1);
}

/**
 * Walk a React child tree and collect all text nodes as a single string.
 * This gives us the raw source code for the clipboard copy.
 */
function extractTextContent(node: unknown): string {
  if (typeof node === 'string') return node;
  if (typeof node === 'number') return String(node);
  if (Array.isArray(node)) return node.map(extractTextContent).join('');
  if (node !== null && typeof node === 'object') {
    const el = node as ReactElement<{ children?: unknown }>;
    if (el.props?.children !== undefined) return extractTextContent(el.props.children);
  }
  return '';
}

// ── Inner CodeBlockPre component ─────────────────────────────────────────────

interface CodeBlockPreProps extends ComponentPropsWithoutRef<'pre'> {
  tone: 'agent' | 'user';
}

function CodeBlockPre({ children, tone, ...rest }: CodeBlockPreProps) {
  const { t } = useT();
  const [copied, setCopied] = useState(false);

  // react-markdown v10 (hast) wraps code in a single <code> child of <pre>.
  // We inspect that child to extract language + raw text.
  const codeChild = Array.isArray(children) ? children[0] : children;
  const codeEl = codeChild as ReactElement<{ className?: string; children?: unknown }> | undefined;
  const codeClassName = codeEl?.props?.className;
  const language = extractLanguage(codeClassName);
  const rawCode = extractTextContent(codeEl?.props?.children).trimEnd();

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(rawCode);
      setCopied(true);
      setTimeout(() => setCopied(false), 1800);
    } catch {
      // Clipboard unavailable (permissions / non-secure context) — no-op.
      console.debug('[CodeBlock] clipboard write failed; user can select-copy manually.');
    }
  };

  // Tailwind classes vary by tone to match the bubble chrome in
  // AgentMessageBubble.tsx. Agent tone: light surfaces; user tone: white
  // overlays on the dark primary bubble.
  const headerBg =
    tone === 'user'
      ? 'bg-white/10 border-b border-white/15'
      : 'bg-stone-200/60 dark:bg-neutral-800/80 border-b border-stone-300/60 dark:border-neutral-700/60';

  const langBadgeClasses =
    tone === 'user'
      ? 'text-white/70 text-[10px] font-mono font-medium uppercase tracking-widest'
      : 'text-content-muted text-[10px] font-mono font-medium uppercase tracking-widest';

  const copyBtnBase = 'text-[11px] font-medium rounded-md px-2.5 py-1 transition-colors';
  const copyBtnIdle =
    tone === 'user'
      ? `${copyBtnBase} bg-white/10 hover:bg-white/20 text-white/80 hover:text-white`
      : `${copyBtnBase} bg-stone-300/50 hover:bg-stone-300/80 dark:bg-neutral-700/60 dark:hover:bg-neutral-700 text-content-muted hover:text-content`;
  const copyBtnDone =
    tone === 'user'
      ? `${copyBtnBase} bg-white/20 text-white`
      : `${copyBtnBase} bg-primary-500/15 dark:bg-primary-500/20 text-primary-600 dark:text-primary-400`;

  return (
    // data-tone enables the [data-tone="user"] CSS selectors in code-highlight.css
    // that override hljs token colours on the dark user-bubble background.
    <div data-tone={tone} className="rounded-lg overflow-hidden my-2">
      {/* Header bar: language label + copy button */}
      <div className={`flex items-center justify-between px-3 py-1.5 ${headerBg}`}>
        <span className={langBadgeClasses}>{language ? formatLanguageLabel(language) : ''}</span>
        <button
          type="button"
          onClick={() => void handleCopy()}
          className={copied ? copyBtnDone : copyBtnIdle}
          aria-label={copied ? t('codeBlock.copied', 'Copied!') : t('codeBlock.copy', 'Copy')}>
          {copied ? t('codeBlock.copied', 'Copied!') : t('codeBlock.copy', 'Copy')}
        </button>
      </div>

      {/* Code body — rehype-highlight has already applied hljs classes */}
      <pre
        {...rest}
        className={`${rest.className ?? ''} !my-0 !rounded-none overflow-x-auto px-3 py-2.5 ${
          tone === 'user' ? 'bg-white/10' : 'bg-stone-300/50 dark:bg-neutral-800/60'
        }`}>
        {children}
      </pre>
    </div>
  );
}

// ── Factory ──────────────────────────────────────────────────────────────────

/**
 * Returns a `pre` component bound to `tone`. Memoize in the parent:
 *
 *   const codePre = useMemo(() => createCodeBlockPre(tone), [tone]);
 */
export function createCodeBlockPre(
  tone: 'agent' | 'user'
): (props: ComponentPropsWithoutRef<'pre'>) => ReactElement {
  return function BoundCodeBlockPre(props: ComponentPropsWithoutRef<'pre'>) {
    return <CodeBlockPre {...props} tone={tone} />;
  };
}
