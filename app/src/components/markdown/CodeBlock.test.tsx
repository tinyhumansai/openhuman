/**
 * Tests for CodeBlock.tsx — the custom react-markdown `pre` component that
 * adds a language label + copy-to-clipboard header above highlighted code.
 *
 * Testing conventions follow AgentMessageBubble.test.tsx:
 *   - Render helpers from @testing-library/react.
 *   - No i18n provider needed: I18nContext falls back to English via resolveEn().
 *   - Clipboard is mocked via vi.stubGlobal before each test that exercises it.
 */
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import React from 'react';
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';

import { createCodeBlockPre } from './CodeBlock';

// ── Helpers ─────────────────────────────────────────────────────────────────

/**
 * Build a minimal react-markdown-like <pre> subtree for a fenced code block.
 * react-markdown wraps `<code className="language-xxx">text</code>` inside <pre>.
 */
function makePre(
  code: string,
  language: string | null,
  tone: 'agent' | 'user' = 'agent'
): React.ReactElement {
  const CodeBlockPre = createCodeBlockPre(tone);
  const codeEl = language ? (
    <code className={`language-${language}`}>{code}</code>
  ) : (
    <code>{code}</code>
  );
  return <CodeBlockPre>{codeEl}</CodeBlockPre>;
}

// ── Clipboard mock ─────────────────────────────────────────────────────────

let writeMock: ReturnType<typeof vi.fn>;

beforeEach(() => {
  writeMock = vi.fn().mockResolvedValue(undefined);
  vi.stubGlobal('navigator', { ...navigator, clipboard: { writeText: writeMock } });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

// ── Tests ────────────────────────────────────────────────────────────────────

describe('CodeBlock — language label', () => {
  test('renders the language label when a language class is present', () => {
    render(makePre('const x = 1;', 'typescript'));
    // Short labels are uppercased; TS stays "TYPESCRIPT" (>4 chars → title-case)
    expect(screen.getByText('Typescript')).toBeInTheDocument();
  });

  test('shows short language tags in uppercase', () => {
    render(makePre('SELECT 1;', 'sql'));
    expect(screen.getByText('SQL')).toBeInTheDocument();
  });

  test('renders without crashing and shows copy button when no language class is present', () => {
    render(makePre('plain text block', null));
    // No language label text, but the copy button must still render.
    expect(screen.getByRole('button', { name: /copy/i })).toBeInTheDocument();
  });
});

describe('CodeBlock — copy button', () => {
  test('calls clipboard.writeText with the raw code text', async () => {
    const sampleCode = 'function greet() { return "hello"; }';
    render(makePre(sampleCode, 'javascript'));

    await userEvent.click(screen.getByRole('button', { name: /copy/i }));

    await waitFor(() => expect(writeMock).toHaveBeenCalledWith(sampleCode));
  });

  test('transitions to "Copied!" state after a successful copy', async () => {
    render(makePre('const value = 42;', 'typescript'));

    const btn = screen.getByRole('button', { name: /copy/i });
    await userEvent.click(btn);

    await waitFor(() =>
      expect(screen.getByRole('button', { name: /copied/i })).toBeInTheDocument()
    );
  });

  test('copy still works on a no-language block', async () => {
    const content = 'untagged code block content';
    render(makePre(content, null));

    await userEvent.click(screen.getByRole('button', { name: /copy/i }));

    await waitFor(() => expect(writeMock).toHaveBeenCalledWith(content));
  });

  test('does not throw when clipboard is unavailable', async () => {
    writeMock.mockRejectedValueOnce(new DOMException('Not allowed', 'NotAllowedError'));
    const consoleSpy = vi.spyOn(console, 'debug').mockImplementation(() => undefined);

    render(makePre('some code', 'python'));
    await userEvent.click(screen.getByRole('button', { name: /copy/i }));

    // Should remain in the non-copied state and have logged a debug message.
    await waitFor(() =>
      expect(consoleSpy).toHaveBeenCalledWith(expect.stringContaining('[CodeBlock]'))
    );
    consoleSpy.mockRestore();
  });
});

describe('CodeBlock — user tone', () => {
  test('renders with data-tone="user" on the container', () => {
    const { container } = render(makePre('print("hi")', 'python', 'user'));
    expect(container.querySelector('[data-tone="user"]')).not.toBeNull();
  });
});
