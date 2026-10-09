import { type ReactNode, useMemo } from 'react';
import Markdown from 'react-markdown';
import rehypeHighlight from 'rehype-highlight';
import rehypeKatex from 'rehype-katex';
import remarkGfm from 'remark-gfm';
import remarkMath from 'remark-math';

import { createCodeBlockPre } from '../../../components/markdown/CodeBlock';
import { hasLatexContent, normalizeLatexDelimiters } from '../../../utils/latex';
import { openUrl } from '../../../utils/openUrl';
import { openWorkspacePath } from '../../../utils/tauriCommands/workspacePaths';
import { parseWorkspaceHref } from '../../../utils/workspaceLinks';
import { ExternalSchemeLink } from '../tools/mcpUi/LinkActions';
import { classifyHref, isAllowedExternalHref, transformChatUrl } from '../utils/format';

const GFM_REMARK_PLUGINS = [remarkGfm];
const MATH_REMARK_PLUGINS = [remarkGfm, remarkMath];
// rehype-highlight must come before rehypeKatex so code blocks inside math
// environments are not double-processed.
const HIGHLIGHT_REHYPE_PLUGINS = [rehypeHighlight];
const MATH_REHYPE_PLUGINS = [rehypeHighlight, rehypeKatex];

function MarkdownAnchor({ href, children }: { href?: string; children?: ReactNode }) {
  if (href && classifyHref(href) === 'handoff') {
    return <ExternalSchemeLink href={href}>{children}</ExternalSchemeLink>;
  }
  return (
    <a
      href={href}
      onClick={e => {
        e.preventDefault();
        const workspaceTarget = parseWorkspaceHref(href);
        if (workspaceTarget) {
          void openWorkspacePath(workspaceTarget.path).catch(err => {
            console.error('workspace open failed:', err);
          });
          return;
        }
        if (!href || !isAllowedExternalHref(href)) return;
        void openUrl(href).catch(() => {
          // Ignore launcher errors from OS URL handler failures.
        });
      }}
      className="cursor-pointer underline wrap-break-word wrap-anywhere">
      {children}
    </a>
  );
}

export function BubbleMarkdown({
  content,
  tone = 'agent',
}: {
  content: string;
  tone?: 'agent' | 'user';
}) {
  const proseTone =
    tone === 'user'
      ? 'prose-invert prose-p:text-content-inverted prose-li:text-content-inverted prose-a:text-content-inverted prose-code:text-content-inverted prose-strong:text-content-inverted prose-headings:text-content-inverted [&_li::marker]:text-content-inverted/85'
      : 'dark:prose-invert prose-a:text-primary-500 prose-code:text-primary-700 dark:prose-code:text-primary-300 prose-headings:text-sm [&_li::marker]:text-content-secondary';

  const hasMath = hasLatexContent(content);
  const rendered = hasMath ? normalizeLatexDelimiters(content) : content;

  // Memoize the `pre` override so it stays reference-stable across re-renders
  // that don't change tone (avoids remounting code blocks on every keystroke).
  const markdownComponents = useMemo(
    () => ({ a: MarkdownAnchor, pre: createCodeBlockPre(tone) }),
    [tone]
  );

  return (
    // prose-pre:my-2 and prose-pre:rounded-lg are kept to style the outer
    // wrapper div emitted by CodeBlock.tsx (it is not a <pre> itself, but
    // Tailwind prose targets the <pre> inside it via the child selector).
    // prose-pre:bg-* classes are intentionally removed: CodeBlock owns the
    // background colours on both the header bar and the code body.
    <div
      className={`text-sm prose prose-sm max-w-none prose-p:my-1 prose-pre:my-0 prose-code:text-xs prose-headings:font-semibold prose-ul:my-0 prose-ol:my-0 prose-li:my-0 ${proseTone} [&_ul]:my-0 [&_ol]:my-0 [&_ul]:pl-0 [&_ol]:pl-0 [&_ul]:list-inside [&_ol]:list-inside [&_li]:my-0 [&_li]:pl-0 [&_li_p]:inline [&_li_p]:m-0`}>
      <Markdown
        urlTransform={transformChatUrl}
        components={markdownComponents}
        remarkPlugins={hasMath ? MATH_REMARK_PLUGINS : GFM_REMARK_PLUGINS}
        rehypePlugins={hasMath ? MATH_REHYPE_PLUGINS : HIGHLIGHT_REHYPE_PLUGINS}>
        {rendered}
      </Markdown>
    </div>
  );
}
