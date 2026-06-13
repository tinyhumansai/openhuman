/**
 * Normalize LaTeX math delimiters emitted by upstream LLMs into the
 * `$...$` / `$$...$$` form that `remark-math` understands.
 *
 * Models frequently emit `\[ ... \]` (display) and `\( ... \)` (inline)
 * or even bare `[ ... ]` blocks containing `\begin{vmatrix}`, `\cdot`,
 * `x_1`, etc. Without this normalization those land in chat as raw
 * source instead of rendered math.
 */

const DISPLAY_BACKSLASH = /\\\[([\s\S]+?)\\\]/g;
const INLINE_BACKSLASH = /\\\(([\s\S]+?)\\\)/g;

// Bare `[ ... ]` block that contains a LaTeX-only signal (\begin, \cdot,
// \times, etc.) and lives on its own line. Conservative: avoids matching
// markdown link/image syntax (`[text](url)`, `![alt](src)`).
const DISPLAY_BARE_BRACKETS =
  /(^|\n)[ \t]*\[[ \t]*((?:[^[\]\n]|\n(?!\n))*?\\(?:begin|end|frac|sqrt|cdot|times|sum|int|prod|lim|left|right|vmatrix|pmatrix|bmatrix|matrix|mathrm|mathbf|mathbb|alpha|beta|gamma|delta|theta|pi|sigma|infty)[^[\]]*?)[ \t]*\][ \t]*(?=\n|$)/g;

// Match fenced code blocks (```...```) and inline code spans (`...`) so
// they can be masked out before delimiter normalization. Without this,
// content like "use `\[x^2\]` for display math" would get its inline
// code corrupted into `$$x^2$$`.
const CODE_BLOCKS = /```[\s\S]*?```|`[^`\n]+`/g;

// Sentinel uses Unicode Private Use Area (U+E000 / U+E001):
//  - non-control (ESLint `no-control-regex` does not fire)
//  - reserved by Unicode for private use, will not appear in real chat text
//  - wrapped on both sides so the restore regex matches *only* placeholders
//    and never stray digits inside math bodies (e.g. `$$a^2$$`).
const PLACEHOLDER_OPEN = '';
const PLACEHOLDER_CLOSE = '';
const PLACEHOLDER = /(\d+)/g;

export function normalizeLatexDelimiters(input: string): string {
  if (!input || (!input.includes('\\') && !input.includes('['))) return input;

  const codeSegments: string[] = [];
  let out = input.replace(CODE_BLOCKS, match => {
    codeSegments.push(match);
    return `${PLACEHOLDER_OPEN}${codeSegments.length - 1}${PLACEHOLDER_CLOSE}`;
  });

  out = out.replace(DISPLAY_BACKSLASH, (_m, body) => `\n\n$$${body}$$\n\n`);
  out = out.replace(INLINE_BACKSLASH, (_m, body) => `$${body}$`);
  out = out.replace(DISPLAY_BARE_BRACKETS, (_m, lead, body) => `${lead}\n$$${body}$$\n`);

  if (codeSegments.length > 0) {
    out = out.replace(PLACEHOLDER, (_m, i) => codeSegments[Number(i)] ?? '');
  }
  return out;
}

/**
 * Heuristic: does this string likely contain LaTeX math?
 *
 * We use this to gate `remark-math` + `rehype-katex` so plain chat
 * messages (e.g. "$10 vs $20", "[link](url)") are never reinterpreted as
 * math. Only content the LLM clearly intended as math turns the plugins
 * on.
 */
const LATEX_SIGNATURE =
  /\\(?:begin|end|frac|sqrt|cdot|times|sum|int|prod|lim|left|right|vmatrix|pmatrix|bmatrix|matrix|mathrm|mathbf|mathbb|alpha|beta|gamma|delta|theta|pi|sigma|infty)\b|\\\[|\\\(|\$\$/;

export function hasLatexContent(input: string): boolean {
  if (!input) return false;
  return LATEX_SIGNATURE.test(input);
}
