#!/usr/bin/env -S pnpm exec tsx

import { promises as fs } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';

const __filename = fileURLToPath(import.meta.url);
const ROOT = path.resolve(path.dirname(__filename), '..');
const APP_SRC = path.join(ROOT, 'app/src');
const ATTRIBUTE_NAMES = new Set(['aria-label', 'placeholder', 'title', 'alt', 'label']);
const IGNORED_FILE_SUFFIXES = ['.test.tsx', '.test.ts', '.stories.tsx'];
const IGNORED_ANCESTOR_TAGS = new Set(['code', 'pre', 'kbd']);
const IGNORED_TEXT = new Set([
  '·',
  '…',
  'Tab',
  'LLM',
  'MCP',
  'QR',
  'URL',
  'API',
  'OpenHuman',
  'Gmail',
  'Discord',
  'Telegram',
  'iPhone',
]);
const IGNORED_SHORT_TOKENS = new Set([
  'v',
  'x',
  'ms',
  'min',
  'tok',
  'at',
  'of',
  'or',
  'and',
  'to',
  'identit',
]);

interface Finding {
  file: string;
  line: number;
  kind: 'jsx-text' | 'jsx-attr';
  preview: string;
}

async function walk(dir: string, out: string[]): Promise<void> {
  const entries = await fs.readdir(dir, { withFileTypes: true });
  for (const entry of entries) {
    const fullPath = path.join(dir, entry.name);
    if (fullPath.includes('__tests__') || fullPath.includes('/lib/i18n/')) continue;
    if (entry.isDirectory()) {
      await walk(fullPath, out);
      continue;
    }
    if (
      entry.isFile() &&
      fullPath.endsWith('.tsx') &&
      !IGNORED_FILE_SUFFIXES.some(suffix => fullPath.endsWith(suffix))
    ) {
      out.push(fullPath);
    }
  }
}

function normalize(value: string): string {
  return value.replace(/\s+/g, ' ').trim();
}

function shouldReportText(raw: string): boolean {
  const value = normalize(raw);
  if (!value) return false;
  if (IGNORED_TEXT.has(value)) return false;
  if (/^&[a-z]+;$/i.test(value)) return false;
  if (/^&(?:nbsp|middot);/i.test(value)) return false;
  if (value.startsWith('·') || value.startsWith('•')) return false;
  if (IGNORED_SHORT_TOKENS.has(value.toLowerCase())) return false;
  if (!/[A-Za-z]/.test(value)) return false;
  if (/^\d+\s*(GB|MB|KB)$/i.test(value)) return false;
  if (/^[a-z0-9.-]+\.[a-z]{2,}$/i.test(value)) return false;
  if (/^[A-Za-z0-9_.:/-]+\(\)$/.test(value)) return false;
  if (/^[A-Za-z0-9_.:/-]+\.\.\.$/.test(value)) return false;
  if (/^[A-Za-z0-9_.:/-]+\s*[•·]\s*[A-Za-z0-9_.:/-]+$/.test(value)) return false;
  if (/^[A-Z0-9_./:-]+$/.test(value)) return false;
  if (/^[A-Za-z0-9_./:-]+$/.test(value) && !/[A-Z]/.test(value) && value.length <= 3) return false;
  if (value === 'at (' || value === 'of $') return false;
  if (/<[A-Za-z]/.test(value)) return false;
  return true;
}

function getJsxTagName(node: ts.Node): string | null {
  if (ts.isJsxElement(node)) {
    const tag = node.openingElement.tagName;
    return ts.isIdentifier(tag) ? tag.text : null;
  }
  if (ts.isJsxSelfClosingElement(node)) {
    const tag = node.tagName;
    return ts.isIdentifier(tag) ? tag.text : null;
  }
  return null;
}

function hasIgnoredAncestorTag(node: ts.Node): boolean {
  let current: ts.Node | undefined = node.parent;
  while (current) {
    const tagName = getJsxTagName(current);
    if (tagName && IGNORED_ANCESTOR_TAGS.has(tagName)) return true;
    current = current.parent;
  }
  return false;
}

function isInsideTranslateCall(node: ts.Node): boolean {
  let current: ts.Node | undefined = node.parent;
  while (current) {
    if (
      ts.isCallExpression(current) &&
      ts.isIdentifier(current.expression) &&
      current.expression.text === 't'
    ) {
      return true;
    }
    current = current.parent;
  }
  return false;
}

function record(findings: Finding[], sourceFile: ts.SourceFile, node: ts.Node, kind: Finding['kind'], preview: string) {
  const { line } = sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile));
  findings.push({
    file: sourceFile.fileName,
    line: line + 1,
    kind,
    preview: normalize(preview),
  });
}

function visit(sourceFile: ts.SourceFile, findings: Finding[], node: ts.Node): void {
  if (
    ts.isJsxText(node) &&
    !hasIgnoredAncestorTag(node) &&
    shouldReportText(node.getText(sourceFile))
  ) {
    record(findings, sourceFile, node, 'jsx-text', node.getText(sourceFile));
  }

  if (ts.isJsxAttribute(node) && ATTRIBUTE_NAMES.has(node.name.text)) {
    if (hasIgnoredAncestorTag(node)) {
      ts.forEachChild(node, child => visit(sourceFile, findings, child));
      return;
    }
    if (node.initializer && ts.isStringLiteral(node.initializer) && shouldReportText(node.initializer.text)) {
      record(findings, sourceFile, node.initializer, 'jsx-attr', `${node.name.text}="${node.initializer.text}"`);
    }

    if (
      node.initializer &&
      ts.isJsxExpression(node.initializer) &&
      node.initializer.expression &&
      ts.isNoSubstitutionTemplateLiteral(node.initializer.expression) &&
      shouldReportText(node.initializer.expression.text) &&
      !isInsideTranslateCall(node.initializer.expression)
    ) {
      record(
        findings,
        sourceFile,
        node.initializer.expression,
        'jsx-attr',
        `${node.name.text}="${node.initializer.expression.text}"`
      );
    }
  }

  ts.forEachChild(node, child => visit(sourceFile, findings, child));
}

async function main(): Promise<void> {
  const files: string[] = [];
  await walk(APP_SRC, files);
  const findings: Finding[] = [];

  for (const file of files) {
    const source = await fs.readFile(file, 'utf8');
    const sourceFile = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
    visit(sourceFile, findings, sourceFile);
  }

  findings.sort((a, b) => a.file.localeCompare(b.file) || a.line - b.line);

  if (findings.length === 0) {
    console.log('No non-i18nized React UI strings found.');
    return;
  }

  for (const finding of findings) {
    console.log(`${path.relative(ROOT, finding.file)}:${finding.line} [${finding.kind}] ${finding.preview}`);
  }

  process.exitCode = 1;
}

void main().catch(error => {
  console.error(error);
  process.exit(1);
});
