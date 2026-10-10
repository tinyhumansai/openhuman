import test from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync, unlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import { extractAnchors, spliceSnippets, lintEmbedMarkdown, checkLocalLinks, exampleMetadata, exportedNames, runtimeBuilderSetters, proseDescription, generateEmbedDocs } from '../generate-embed-docs.mjs';

const example = '//! Title: Hello\n//! Summary: Builds an offline agent.\n//! Run: offline\n// ANCHOR: hello\nfn main() {}\n// ANCHOR_END: hello\n';
const marker = '<!-- BEGIN EMBED: crates/openhuman-embed/examples/hello.rs#hello -->\nstale\n<!-- END EMBED -->';

test('extracts real source, excludes marker comments, validates anchors', () => {
  assert.equal(extractAnchors(example).get('hello'), 'fn main() {}');
  for (const source of ['// ANCHOR: a', '// ANCHOR_END: a', '// ANCHOR: a\n// ANCHOR_END: a', '// ANCHOR: a\nx\n// ANCHOR_END: a\n// ANCHOR: a', '// ANCHOR: a\n# hidden\n// ANCHOR_END: a']) assert.throws(() => extractAnchors(source));
});
test('injects plain rust fences and changes when the compiled source changes', () => {
  assert.equal(spliceSnippets(marker, () => example), marker.replace('stale', '\n```rust\nfn main() {}\n```\n'));
  assert.match(spliceSnippets(marker, () => example.replace('fn main() {}', 'fn main() { println!("new"); }')), /println!/);
  assert.throws(() => spliceSnippets(marker, () => example.replaceAll('hello', 'other')), /Missing anchor/);
  assert.throws(() => spliceSnippets(marker.replace('examples/hello', 'examples/../hello'), () => example), /must come from/);
  assert.throws(() => spliceSnippets(marker.replace('<!-- END EMBED -->', ''), () => example), /Unclosed/);
});
test('rejects manually typed rust fences, alternative fence style, and hidden lines', () => {
  for (const source of ['```rust\nx\n```', '~~~rust,no_run\nx\n~~~', '```text\n# hidden\n```', '```text\nx']) assert.throws(() => lintEmbedMarkdown(source));
  assert.doesNotThrow(() => lintEmbedMarkdown(spliceSnippets(marker, () => example)));
  assert.doesNotThrow(() => lintEmbedMarkdown('# Title\n\n```text\nnormal\n```'));
});
test('local links verify files and headings without following external services', () => {
  const read = path => { if (path === '/repo/other.md') return '# Some API\n'; throw Error('missing'); };
  assert.doesNotThrow(() => checkLocalLinks('# Here\n[x](other.md#some-api) [external](https://example.com) [self](#here)', '/repo/doc.md', '/repo', read));
  assert.throws(() => checkLocalLinks('[x](missing.md)', '/repo/doc.md', '/repo', read), /Broken local/);
  assert.throws(() => checkLocalLinks('[x](other.md#missing)', '/repo/doc.md', '/repo', read), /Broken heading/);
  assert.throws(() => checkLocalLinks('[x](../secret)', '/repo/doc.md', '/repo', read), /escapes/);
  assert.doesNotThrow(() => checkLocalLinks('```rust\nlet x = "[x](missing.md)";\n```', '/repo/doc.md', '/repo', read));
});
test('cookbook metadata must come from example headers', () => {
  assert.deepEqual(exampleMetadata(example, 'hello'), { name: 'hello', title: 'Hello', summary: 'Builds an offline agent.', run: 'offline', feature: undefined });
  assert.throws(() => exampleMetadata('fn main() {}', 'bad'), /headers/);
});
test('cookbook benchmark commands use the declared release profile without default features', () => {
  const root = mkdtempSync(resolve(tmpdir(), 'embed-benchmark-docs-'));
  try {
    mkdirSync(resolve(root, 'gitbooks/developing/embed'), { recursive: true });
    mkdirSync(resolve(root, 'crates/openhuman-embed/examples'), { recursive: true });
    mkdirSync(resolve(root, 'crates/openhuman-embed/src'), { recursive: true });
    writeFileSync(resolve(root, 'gitbooks/developing/embed/index.md'), '# Benchmarks\n');
    writeFileSync(resolve(root, 'crates/openhuman-embed/src/lib.rs'), 'pub use runtime::Runtime;');
    writeFileSync(resolve(root, 'crates/openhuman-embed/examples/retained_fleet.rs'), example + '//! Profile: release\n//! Feature: default\n//! Default features: disabled\n');
    writeFileSync(resolve(root, 'crates/openhuman-embed/examples/hello.rs'), example);
    const capabilities = resolve(root, 'capabilities.json');
    writeFileSync(capabilities, '{"schema_version":1,"compiled_features":{}}');
    generateEmbedDocs({ root, capabilities });
    const cookbook = readFileSync(resolve(root, 'gitbooks/developing/embed/cookbook.md'), 'utf8');
    assert.match(cookbook, /Run: `cargo run -p openhuman-embed --release --example retained_fleet --no-default-features`/);
    assert.match(cookbook, /Run: `cargo run -p openhuman-embed --example hello`/);
    assert.doesNotMatch(cookbook, /--features none/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
test('generate then check is stable, edited source fails check, no prose authored by tooling', () => {
  const root = mkdtempSync(resolve(tmpdir(), 'embed-docs-'));
  try {
    mkdirSync(resolve(root, 'gitbooks/developing/embed'), { recursive: true });
    mkdirSync(resolve(root, 'crates/openhuman-embed/examples'), { recursive: true });
    mkdirSync(resolve(root, 'crates/openhuman-embed/src/runtime'), { recursive: true });
    const builder = resolve(root, 'crates/openhuman-embed/src/runtime/builder.rs');
    writeFileSync(builder, 'impl RuntimeBuilder { pub fn max_agents(mut self, limit: usize) -> Self { self } }');
    writeFileSync(resolve(root, 'crates/openhuman-embed/src/lib.rs'), 'pub use runtime::{Runtime, RuntimeBuilder};');
    const path = resolve(root, 'crates/openhuman-embed/examples/hello.rs');
    writeFileSync(path, example);
    writeFileSync(resolve(root, 'gitbooks/developing/embed/index.md'), '---\ndescription: Metadata description\nicon: code\n---\n\n# Introduction\n\nThe actual introduction explains the host.\n\n' + marker + '\n');
    mkdirSync(resolve(root, 'docs/gitbooks/en/developing'), { recursive: true });
    writeFileSync(resolve(root, 'gitbooks/developing/embedding.md'), '# Compatibility\n\n[Introduction](embed/index.md)\n');
    writeFileSync(resolve(root, 'docs/gitbooks/en/developing/embedding.md'), '---\ndescription: >-\n  Preserved existing GitBook metadata.\nicon: code\nlayout:\n  width: wide\n---\n\n# Old pointer\n');
    writeFileSync(resolve(root, 'gitbooks/developing/embed/retired.md'), '# Retired page\n\nA temporary canonical page.\n');
    const capabilities = resolve(root, 'capabilities.json');
    writeFileSync(capabilities, '{"schema_version":1,"compiled_features":{"mcp":true,"channels":false}}');
    generateEmbedDocs({ root, capabilities });
    generateEmbedDocs({ root, capabilities, check: true });
    assert.match(readFileSync(resolve(root, 'llms-full.txt'), 'utf8'), /fn main\(\)/);
    const published = resolve(root, 'docs/gitbooks/en/developing/embed/index.md');
    assert.equal(readFileSync(published, 'utf8'), readFileSync(resolve(root, 'gitbooks/developing/embed/index.md'), 'utf8'));
    assert.equal(readFileSync(resolve(root, 'docs/gitbooks/en/developing/embed/api-index.json'), 'utf8'), readFileSync(resolve(root, 'gitbooks/developing/embed/api-index.json'), 'utf8'));
    assert.match(readFileSync(resolve(root, 'docs/gitbooks/en/developing/embedding.md'), 'utf8'), /Preserved existing GitBook metadata[\s\S]*width: wide[\s\S]*# Compatibility/);
    const llms = readFileSync(resolve(root, 'llms.txt'), 'utf8');
    assert.match(llms, /Introduction.*The actual introduction explains the host/);
    assert.doesNotMatch(llms, /docs\/gitbooks\/en|Metadata description/);
    assert.equal((readFileSync(resolve(root, 'llms-full.txt'), 'utf8').match(/# Introduction/g) ?? []).length, 1);
    assert.match(readFileSync(resolve(root, 'gitbooks/developing/embed/api-index.md'), 'utf8'), /cargo doc.*unpublished/);
    assert.doesNotMatch(readFileSync(resolve(root, 'gitbooks/developing/embed/api-index.md'), 'utf8'), /https:\/\/docs[.]rs/);
    writeFileSync(published, readFileSync(published, 'utf8').replace('actual introduction', 'mirror drift'));
    assert.throws(() => generateEmbedDocs({ root, capabilities, check: true }), /docs\/gitbooks\/en\/developing\/embed\/index[.]md/);
    generateEmbedDocs({ root, capabilities });
    assert.match(readFileSync(resolve(root, 'gitbooks/developing/embed/api-index.md'), 'utf8'), /schema_version/);
    assert.deepEqual(JSON.parse(readFileSync(resolve(root, 'gitbooks/developing/embed/api-index.json'), 'utf8')).exports, ['Runtime', 'RuntimeBuilder']);
    assert.match(readFileSync(resolve(root, 'crates/openhuman-embed/examples/README.md'), 'utf8'), /Cookbook/);
    assert.match(readFileSync(resolve(root, 'gitbooks/developing/embed/builder-setters.md'), 'utf8'), /pub fn max_agents/);
    assert.match(readFileSync(resolve(root, 'gitbooks/developing/embed/builder-setters.md'), 'utf8'), /https:\/\/github\.com\/tinyhumansai\/openhuman\/blob\/main\/crates\/openhuman-embed\/src\/runtime\/builder\.rs#L1/);
    assert.match(readFileSync(resolve(root, 'gitbooks/developing/embed/cookbook.md'), 'utf8'), /https:\/\/github\.com\/tinyhumansai\/openhuman\/blob\/main\/crates\/openhuman-embed\/examples\/hello\.rs/);
    writeFileSync(builder, 'impl RuntimeBuilder { pub fn max_agents(mut self, limit: usize) -> Self { self } pub fn new_knob(self, enabled: bool) -> Self { self } }');
    assert.throws(() => generateEmbedDocs({ root, capabilities, check: true }), /drift/);
    generateEmbedDocs({ root, capabilities });
    assert.match(readFileSync(resolve(root, 'gitbooks/developing/embed/builder-setters.md'), 'utf8'), /new_knob/);
    writeFileSync(builder, 'impl RuntimeBuilder { pub fn new_knob(self, enabled: bool) -> Self { self } }');
    assert.throws(() => generateEmbedDocs({ root, capabilities, check: true }), /drift/);
    generateEmbedDocs({ root, capabilities });
    assert.doesNotMatch(readFileSync(resolve(root, 'gitbooks/developing/embed/builder-setters.md'), 'utf8'), /max_agents/);
    const matrix = readFileSync(resolve(root, 'gitbooks/developing/embed/capability-matrix.md'), 'utf8');
    assert.match(matrix, /channels` \| No/);
    assert.match(matrix, /mcp` \| Yes/);
    writeFileSync(capabilities, '{"schema_version":1,"compiled_features":{"mcp":false,"channels":false}}');
    assert.throws(() => generateEmbedDocs({ root, capabilities, check: true }), /drift/);
    generateEmbedDocs({ root, capabilities });
    generateEmbedDocs({ root, capabilities, check: true });
    writeFileSync(path, example.replace('fn main() {}', 'fn main() { println!("changed"); }'));
    assert.throws(() => generateEmbedDocs({ root, capabilities, check: true }), error => {
      assert.match(error.message, /gitbooks\/developing\/embed\/index[.]md/);
      assert.match(error.message, /docs\/gitbooks\/en\/developing\/embed\/index[.]md/);
      return true;
    });
    generateEmbedDocs({ root, capabilities });
    generateEmbedDocs({ root, capabilities, check: true });
    assert.match(readFileSync(published, 'utf8'), /println!\("changed"\)/);
    unlinkSync(resolve(root, 'gitbooks/developing/embed/retired.md'));
    assert.throws(() => generateEmbedDocs({ root, capabilities, check: true }), /docs\/gitbooks\/en\/developing\/embed\/retired[.]md/);
    generateEmbedDocs({ root, capabilities });
    assert.equal(existsSync(resolve(root, 'docs/gitbooks/en/developing/embed/retired.md')), false);
    generateEmbedDocs({ root, capabilities, check: true });
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('API index includes public export aliases and excludes internal facade', () => {
  assert.deepEqual(exportedNames('pub mod __host;\npub mod process;\npub mod providers { }\n#[doc(hidden)]\npub use runtime::BuilderSummary;\npub use runtime::{Runtime, RuntimeBuilder};\npub use a::Config as Configuration;'), ['Configuration', 'Runtime', 'RuntimeBuilder', 'process', 'providers']);
});


test('builder setters include real consuming signatures and exclude presets, other types and strings', () => {
  const source = `impl Runtime { pub fn wrong(self) -> Self { self } }
impl RuntimeBuilder {
  pub fn library() -> Self { Self::new() }
  pub fn describe(&self) -> Self { self.clone() }
  pub fn build(self) -> Result<Self> { todo!() }
  pub(crate) fn hidden(self) -> Self { self }
  pub fn actual(
      mut self,
      hook: impl Fn(u32) -> u32,
  ) -> Self { let text = "pub fn bogus(self) -> Self { }"; self }
  // pub fn commented(self) -> Self { self }
}
impl RuntimeBuilder { pub fn another(self, path: impl Into<PathBuf>) -> Self { self } }`;
  const setters = runtimeBuilderSetters(source, 'builder.rs');
  assert.deepEqual(setters.map(setter => setter.name), ['actual', 'another']);
  assert.equal(setters[0].signature, 'pub fn actual( mut self, hook: impl Fn(u32) -> u32, ) -> Self');
  assert.equal(setters[0].source, 'builder.rs');
  assert.equal(setters[0].line, 7);
});


test('builder parser masks multiline Rust literals and raw strings without losing braces', () => {
  const source = String.raw`impl RuntimeBuilder {
    pub fn knob(self, name: &'static str) -> Self {
      let raw = r###"{ \" nested quotes and // data"###;
      let continued = "{ \
        continued string";
      let character = '{';
      self
    }
  }`;
  assert.deepEqual(runtimeBuilderSetters(source, 'source.rs').map(setter => setter.name), ['knob']);
});


test('index descriptions select prose rather than metadata, snippets or generation markers', () => {
  assert.equal(proseDescription('---\ndescription: >-\n  Metadata\nicon: code\n---\n# Title\n\n<!-- generated -->\n```rust\nfn main() {}\n```\n\nThe [host](host.md) owns transport.\n'), 'The host owns transport.');
  assert.equal(proseDescription('# Title\n\n| API | Scope |\n| --- | --- |\n- [link](foo.md)\n\nActual introduction.'), 'Actual introduction.');
});
