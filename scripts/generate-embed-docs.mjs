#!/usr/bin/env node
/** Embed documentation is generated only from compiled examples and runtime introspection. */
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, extname, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const MARKER = /^<!-- BEGIN EMBED: ([^\s#]+)#([\w-]+) -->$/;

export function extractAnchors(source) {
  const regions = new Map();
  const active = new Map();
  for (const line of source.split('\n')) {
    const start = /^\s*\/\/ ANCHOR: ([\w-]+)\s*$/.exec(line);
    const end = /^\s*\/\/ (?:ANCHOR_END|END ANCHOR): ([\w-]+)\s*$/.exec(line);
    if (start) {
      if (regions.has(start[1]) || active.has(start[1])) throw new Error(`Duplicate anchor: ${start[1]}`);
      active.set(start[1], []);
    } else if (end) {
      if (!active.has(end[1])) throw new Error(`Unmatched anchor end: ${end[1]}`);
      const body = active.get(end[1]).join('\n');
      if (!body.trim()) throw new Error(`Empty anchor: ${end[1]}`);
      if (/^\s*#(?:\s|$)/m.test(body)) throw new Error(`Hidden rustdoc line in anchor: ${end[1]}`);
      regions.set(end[1], body);
      active.delete(end[1]);
    } else {
      for (const lines of active.values()) lines.push(line);
    }
  }
  if (active.size) throw new Error(`Unclosed anchors: ${[...active.keys()].join(', ')}`);
  return regions;
}

export function spliceSnippets(markdown, readSource) {
  const lines = markdown.split('\n');
  const output = [];
  for (let i = 0; i < lines.length; i++) {
    const match = MARKER.exec(lines[i]);
    if (!match) {
      if (lines[i].includes('BEGIN EMBED:') || lines[i] === '<!-- END EMBED -->') throw new Error('Malformed or unmatched embed splice marker');
      output.push(lines[i]);
      continue;
    }
    output.push(lines[i]);
    const source = match[1];
    if (!source.startsWith('crates/openhuman-embed/examples/') || !source.endsWith('.rs') || source.split('/').includes('..')) throw new Error(`Snippet must come from an embed example: ${source}`);
    const body = extractAnchors(readSource(source)).get(match[2]);
    if (body === undefined) throw new Error(`Missing anchor ${source}#${match[2]}`);
    output.push('', '```rust', body, '```', '');
    while (++i < lines.length && lines[i] !== '<!-- END EMBED -->') {
      if (lines[i].includes('BEGIN EMBED:')) throw new Error('Nested embed splice markers');
    }
    if (i === lines.length) throw new Error('Unclosed embed splice marker');
    output.push(lines[i]);
  }
  return output.join('\n');
}

export function lintEmbedMarkdown(markdown) {
  let generated = false;
  let fence = null;
  for (const line of markdown.split('\n')) {
    if (MARKER.test(line)) generated = true;
    if (line === '<!-- END EMBED -->') generated = false;
    const opening = /^\s*(`{3,}|~{3,})(.*)$/.exec(line);
    if (opening) {
      if (fence && opening[1][0] === fence[0] && opening[1].length >= fence.length && !opening[2].trim()) fence = null;
      else if (!fence) {
        const language = opening[2].trim();
        if (/^rust(?:\b|,)/i.test(language) && !generated) throw new Error('Hand-written Rust fence; use an example splice marker');
        fence = opening[1];
      }
    } else if (fence && /^\s*#(?:\s|$)/.test(line)) throw new Error('Hidden rustdoc line in prose code block');
  }
  if (fence) throw new Error('Unclosed Markdown fence');
}

function markdownFiles(directory, extensions = new Set(['.md'])) {
  if (!existsSync(directory)) return [];
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const path = resolve(directory, entry.name);
    return entry.isDirectory() ? markdownFiles(path, extensions) : extensions.has(extname(path)) ? [path] : [];
  }).sort();
}

export function checkLocalLinks(markdown, document, root, read = path => readFileSync(path, 'utf8')) {
  const headings = source => new Set([...source.matchAll(/^#{1,6}\s+(.+)$/gm)].map(match => match[1].toLowerCase().replace(/[`*_]/g, '').replace(/[^\p{L}\p{N}\s-]/gu, '').trim().replace(/\s/g, '-')));
  // Fenced code is data, not Markdown links.
  const prose = markdown.replace(/^\s*(`{3,}|~{3,})[^\n]*\n[\s\S]*?^\s*\1\s*$/gm, '');
  for (const match of prose.matchAll(/!?\[[^\]]*\]\(([^\s)]+)(?:\s+"[^"]*")?\)/g)) {
    const url = match[1].replace(/^<|>$/g, '');
    if (/^(?:[a-z][a-z\d+.-]*:|\/\/)/i.test(url)) continue;
    const [link, fragment] = url.split('#');
    const path = link ? resolve(link.startsWith('/') ? root : dirname(document), decodeURIComponent(link.replace(/^\//, '').split('?')[0])) : document;
    if (path !== root && !path.startsWith(root + sep)) throw new Error(`Link escapes repository: ${url}`);
    let source;
    try { source = path === document ? markdown : read(path); } catch { throw new Error(`Broken local link in ${document}: ${url}`); }
    if (fragment && extname(path) === '.md' && !headings(source).has(decodeURIComponent(fragment))) throw new Error(`Broken heading link in ${document}: ${url}`);
  }
}

export function exampleMetadata(source, name) {
  const field = key => new RegExp(`^//! ${key}: (.+)$`, 'm').exec(source)?.[1];
  const title = field('Title');
  const summary = field('Summary');
  if (!title || !summary) throw new Error(`Example ${name} needs //! Title: and //! Summary: headers`);
  const feature = field('Feature');
  const profile = field('Profile');
  if (profile && !['dev', 'release'].includes(profile)) throw new Error(`Example ${name} has unsupported Profile: ${profile}`);
  return {
    name, title, summary, run: field('Run') ?? 'offline',
    feature: feature === 'default' ? undefined : feature,
    ...(profile ? { profile } : {}),
    ...(field('Default features') === 'disabled' ? { noDefaultFeatures: true } : {}),
  };
}

/** Public facade exports, suitable for rustdoc search links (including aliases). */
export function exportedNames(source) {
  // Doc-hidden compatibility seams are not part of the curated host API.
  source = source.replace(/#\[doc\(hidden\)\]\s*(?:#\[[^\]]+\]\s*)*pub\s+(?:use\s+[\s\S]*?|mod\s+\w+);/g, '');
  const names = new Set();
  for (const match of source.matchAll(/^pub mod ([a-zA-Z_][\w]*)\s*[;{]/gm)) if (!match[1].startsWith('__')) names.add(match[1]);
  for (const match of source.matchAll(/^pub use ([\s\S]*?);/gm)) {
    const body = match[1].replace(/\/\/[^\n]*/g, '');
    const items = body.includes('{') ? body.slice(body.indexOf('{') + 1, body.lastIndexOf('}')).split(',') : [body];
    for (const item of items) {
      const name = /(?:as\s+|::|^)([a-zA-Z_]\w*)\s*$/.exec(item.trim())?.[1];
      if (name && !name.startsWith('__')) names.add(name);
    }
  }
  return [...names].sort();
}

/** Consuming public setters on RuntimeBuilder, excluding presets and borrowed methods. */
export function runtimeBuilderSetters(source, file) {
  // Mask strings and comments without moving offsets used by source links.
  const masked = source.replace(/r(#+)"[\s\S]*?"\1|r"[\s\S]*?"|"(?:\\[\s\S]|[^"\\])*"|'(?:\\[\s\S]|[^'\\\n])'|\/\/[^\n]*|\/\*[\s\S]*?\*\//g, value => value.replace(/[^\n]/g, ' '));
  const setters = [];
  for (const declaration of masked.matchAll(/\bimpl\s+RuntimeBuilder\s*\{/g)) {
    const begin = declaration.index + declaration[0].length;
    let end = begin;
    let depth = 1;
    while (end < masked.length && depth) {
      if (masked[end] === '{') depth++;
      if (masked[end] === '}') depth--;
      end++;
    }
    if (depth) throw new Error(`Unclosed RuntimeBuilder implementation in ${file}`);
    const body = masked.slice(begin, end - 1);
    for (const method of body.matchAll(/\bpub\s+(?:async\s+)?fn\s+(\w+)[^(]*\(/g)) {
      const preceding = body.slice(0, method.index);
      if ([...preceding].reduce((nesting, char) => nesting + (char === '{' ? 1 : char === '}' ? -1 : 0), 0)) continue;
      const open = begin + method.index + method[0].length - 1;
      let close = open + 1;
      let parameters = 1;
      while (close < end && parameters) {
        if (masked[close] === '(') parameters++;
        if (masked[close] === ')') parameters--;
        close++;
      }
      if (parameters) throw new Error(`Unclosed method signature in ${file}`);
      const receiver = masked.slice(open + 1, close - 1).split(',')[0].trim();
      const returned = /^\s*->\s*Self\b/.exec(masked.slice(close));
      if (!/^(?:mut\s+)?self(?:\s*:\s*Self)?$/.test(receiver) || !returned) continue;
      const offset = begin + method.index;
      setters.push({ name: method[1], signature: source.slice(offset, close + returned[0].length).replace(/\s+/g, ' ').trim(), source: file, line: source.slice(0, offset).split('\n').length });
    }
  }
  return setters.sort((left, right) => left.name.localeCompare(right.name));
}

/** Human-facing index descriptions ignore GitBook metadata and code/data blocks. */
export function proseDescription(markdown) {
  const prose = markdown
    .replace(/^---\r?\n[\s\S]*?\r?\n---(?:\r?\n|$)/, '')
    .replace(/^\s*(`{3,}|~{3,})[^\n]*\n[\s\S]*?^\s*\1\s*$/gm, '')
    .replace(/<!--[\s\S]*?-->/g, '');
  return (prose.split('\n').find(line => line.trim() && !/^\s*(?:#|[|]|[-*]\s|\d+[.]\s|[{]%|<)/.test(line)) ?? '')
    .trim().replace(/\[([^\]]+)\]\([^)]+\)/g, '$1');
}

function preserveFrontmatter(source, existing) {
  const prefix = /^---\r?\n[\s\S]*?\r?\n---(?:\r?\n|$)/;
  const metadata = prefix.exec(existing)?.[0];
  return metadata ? metadata + source.replace(prefix, '') : source;
}

export function generateEmbedDocs({ root = ROOT, check = false, capabilities } = {}) {
  const directory = resolve(root, 'gitbooks/developing/embed');
  if (!existsSync(directory)) return; // Track D starts only after code/examples/tooling verification.
  const docs = markdownFiles(directory).filter(path => !['cookbook.md', 'api-index.md', 'capability-matrix.md', 'builder-setters.md'].includes(path.split(sep).at(-1)));
  if (!docs.length) throw new Error('Embed documentation directory is empty');
  const desired = new Map();
  for (const path of ['crates/openhuman-embed/README.md', 'gitbooks/developing/embedding.md'].map(path => resolve(root, path))) {
    if (existsSync(path)) docs.push(path);
  }
  for (const path of docs) {
    const current = readFileSync(path, 'utf8');
    lintEmbedMarkdown(current);
    const updated = spliceSnippets(current, source => readFileSync(resolve(root, source), 'utf8'));
    lintEmbedMarkdown(updated);
    desired.set(path, updated);
  }
  const examples = readdirSync(resolve(root, 'crates/openhuman-embed/examples')).filter(name => name.endsWith('.rs')).sort().map(file => exampleMetadata(readFileSync(resolve(root, 'crates/openhuman-embed/examples', file), 'utf8'), file.slice(0, -3)));
  desired.set(resolve(directory, 'cookbook.md'), '# Cookbook\n\n<!-- Generated by scripts/generate-embed-docs.mjs; do not edit. -->\n\nExecutable recipes are indexed from their source headers. Each entry records the declared run mode and any named Embed feature required by the example. Full source includes the surrounding setup and assertions.\n\n' + examples.map(example => `## ${example.title}\n\n${example.summary}\n\n[Source](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/${example.name}.rs) · ${example.run}${example.feature ? ` · feature: ${example.feature}` : ''}\n\nRun: \`cargo run -p openhuman-embed${example.profile === 'release' ? ' --release' : ''} --example ${example.name}${example.noDefaultFeatures ? ' --no-default-features' : ''}${example.feature ? ` --features ${example.feature}` : ''}\`\n`).join('\n'));
  const report = capabilities ? JSON.parse(readFileSync(capabilities, 'utf8')) : JSON.parse(execFileSync('bash', ['scripts/ci-cancel-aware.sh', 'cargo', 'run', '--quiet', '-p', 'openhuman-embed', '--example', 'capability_report', '--', '--json'], { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] }));
  const exports = exportedNames(readFileSync(resolve(root, 'crates/openhuman-embed/src/lib.rs'), 'utf8'));
  if (!report || typeof report !== 'object' || !Number.isInteger(report.schema_version) || !report.compiled_features || typeof report.compiled_features !== 'object') throw new Error('Capability report must be versioned RuntimeInfo JSON');
  desired.set(resolve(directory, 'capability-matrix.md'), '# Compiled feature matrix\n\n<!-- Generated by scripts/generate-embed-docs.mjs; do not edit. -->\n\nThis matrix comes from the default-feature compiled capability-report example. Runtime presets can narrow enabled domains and services; core Cargo features determine which implementations are available. Named Embed features also gate facade exports: enable `mcp` or `skills` explicitly for their public setters even when core defaults compile those implementations. Embed defaults enable the named `channels` feature. See the installation guide for these separate layers.\n\n| Cargo feature | Compiled in this build |\n| --- | --- |\n' + Object.entries(report.compiled_features).sort(([left], [right]) => left.localeCompare(right)).map(([feature, enabled]) => `| \`${feature}\` | ${enabled ? 'Yes' : 'No'} |`).join('\n') + '\n');
  const runtimeDirectory = resolve(root, 'crates/openhuman-embed/src/runtime');
  const setters = existsSync(runtimeDirectory) ? readdirSync(runtimeDirectory).filter(file => file.endsWith('.rs') && !file.endsWith('_tests.rs')).sort().flatMap(file => {
    const path = resolve(runtimeDirectory, file);
    return runtimeBuilderSetters(readFileSync(path, 'utf8'), relative(root, path).split(sep).join('/'));
  }).sort((left, right) => left.name.localeCompare(right.name)) : [];
  desired.set(resolve(directory, 'builder-setters.md'), '# RuntimeBuilder setters\n\n<!-- Generated by scripts/generate-embed-docs.mjs; do not edit. -->\n\nPublic consuming methods that return `Self`, extracted from the RuntimeBuilder implementations. Static host and weight presets, inspection methods and build/run methods are excluded.\n\n' + setters.map(setter => `- [\`${setter.name}\`](https://github.com/tinyhumansai/openhuman/blob/main/${setter.source}#L${setter.line}): \`${setter.signature}\``).join('\n') + '\n');
  desired.set(resolve(directory, 'api-index.md'), '# API index\n\n<!-- Generated by scripts/generate-embed-docs.mjs; do not edit. -->\n\nGenerate the complete local Rust API reference with `cargo doc -p openhuman-embed --all-features --no-deps --open`. This crate is currently unpublished; docs.rs is a future publication target. Names below are source-listed exports, including conditional Cargo-gated items; local rustdoc with your selected features describes availability. [Canonical facade source](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/src/lib.rs).\n\n' + exports.map(name => `- [\`${name}\`](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/src/lib.rs)`).join('\n') + '\n\nThe compiled capability report describes this build:\n\n```json\n' + JSON.stringify(report, null, 2) + '\n```\n');
  desired.set(resolve(directory, 'api-index.json'), JSON.stringify({
    schema_version: 1,
    exports,
    builder_setters: setters,
    capabilities: report,
    pages: [...desired.keys()].filter(path => path.endsWith('.md')).map(path => relative(root, path).split(sep).join('/')),
  }, null, 2) + '\n');
  desired.set(resolve(root, 'crates/openhuman-embed/examples/README.md'), '# Embed examples\n\n<!-- Generated by scripts/generate-embed-docs.mjs; do not edit. -->\n\n[Cookbook](../../../gitbooks/developing/embed/cookbook.md)\n\n' + examples.map(example => `- [${example.title}](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/${example.name}.rs): ${example.summary} (${example.run}${example.feature ? `; feature: ${example.feature}` : ''})`).join('\n') + '\n');
  const index = '# OpenHuman Embed\n\n> Embed OpenHuman runtimes and independently configured agents in Rust applications.\n\n' + [...desired].filter(([path]) => path.endsWith('.md')).map(([path, source]) => `- [${/^# (.+)$/m.exec(source)?.[1] ?? 'Documentation'}](${relative(root, path).split(sep).join('/')}): ${proseDescription(source)}`).join('\n') + '\n';
  desired.set(resolve(root, 'llms.txt'), index);
  desired.set(resolve(root, 'llms-full.txt'), index + '\n' + [...desired].filter(([path]) => path.endsWith('.md')).map(([path, source]) => `<!-- Source: ${relative(root, path)} -->\n${source}`).join('\n'));
  desired.set(resolve(root, 'EMBED.md'), index + '\nSee the generated [API index](gitbooks/developing/embed/api-index.md) for build capabilities.\n');
  // Aggregate only canonical pages above. Publish their exact generated bodies
  // afterward, so English copies never duplicate llms-full content or index paths.
  const publishedDirectory = resolve(root, 'docs/gitbooks/en/developing/embed');
  for (const [path, source] of [...desired]) {
    if (path.startsWith(directory + sep)) desired.set(resolve(publishedDirectory, relative(directory, path)), source);
  }
  const compatibility = resolve(root, 'gitbooks/developing/embedding.md');
  const publishedCompatibility = resolve(root, 'docs/gitbooks/en/developing/embedding.md');
  if (desired.has(compatibility)) {
    desired.set(publishedCompatibility, preserveFrontmatter(desired.get(compatibility), existsSync(publishedCompatibility) ? readFileSync(publishedCompatibility, 'utf8') : ''));
  }
  for (const [path, source] of desired) {
    if (path.endsWith('.md')) checkLocalLinks(source, path, root, link => desired.get(link) ?? readFileSync(link, 'utf8'));
  }
  const obsolete = markdownFiles(publishedDirectory, new Set(['.md', '.json'])).filter(path => !desired.has(path));
  const stale = [...desired].filter(([path, source]) => !existsSync(path) || readFileSync(path, 'utf8') !== source);
  if (check && (stale.length || obsolete.length)) throw new Error(`Embed documentation drift: ${[...stale.map(([path]) => path), ...obsolete].map(path => relative(root, path)).join(', ')}. Run pnpm docs:generate.`);
  if (!check) for (const [path, source] of stale) {
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, source);
  }
  if (!check) for (const path of obsolete) rmSync(path);
}

if (resolve(process.argv[1] ?? '') === fileURLToPath(import.meta.url)) {
  try { generateEmbedDocs({ check: process.argv.includes('--check'), capabilities: process.argv.includes('--capabilities') ? process.argv[process.argv.indexOf('--capabilities') + 1] : undefined }); }
  catch (error) { console.error(error.message); process.exitCode = 1; }
}
