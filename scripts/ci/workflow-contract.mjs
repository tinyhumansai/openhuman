// Small, dependency-free readers for the workflow contract tests. They parse
// only the shapes our workflows use, and throw instead of returning an empty
// result, so a renamed job or an unusual runner spelling fails a test rather
// than letting its assertions pass vacuously.

const HOSTED_LABEL = /^(ubuntu|windows|macos)-[\w.-]+$/;

/**
 * The text of one top-level job: from its `  <key>:` line to the next job key.
 * Throws when the workflow has no such job.
 */
export function jobBlock(yaml, key) {
  const lines = yaml.split("\n");
  const jobsAt = lines.findIndex((line) => /^jobs:\s*$/.test(line));
  if (jobsAt < 0) throw new Error("workflow has no jobs: section");
  const start = lines.findIndex(
    (line, i) => i > jobsAt && line.startsWith(`  ${key}:`),
  );
  if (start < 0) throw new Error(`workflow has no job "${key}"`);
  let end = lines.length;
  for (let i = start + 1; i < lines.length; i += 1) {
    if (/^  [A-Za-z0-9_-]+:/.test(lines[i]) || /^\S/.test(lines[i])) {
      end = i;
      break;
    }
  }
  return lines.slice(start, end).join("\n");
}

/**
 * Asserts every job runs on a GitHub-hosted label (ubuntu-*, windows-*,
 * macos-*). Rejects self-hosted, runner groups, arrays, objects, expressions,
 * and jobs without a runs-on.
 */
export function assertGithubHostedRunners(yaml) {
  const lines = yaml.split("\n");
  const jobsAt = lines.findIndex((line) => /^jobs:\s*$/.test(line));
  if (jobsAt < 0) throw new Error("workflow has no jobs: section");
  const keys = lines
    .slice(jobsAt + 1)
    .map((line) => line.match(/^  ([A-Za-z0-9_-]+):/)?.[1])
    .filter(Boolean);
  for (const key of keys) {
    const block = jobBlock(yaml, key);
    const uses = /^    uses:/m.test(block);
    const match = block.match(/^    runs-on:[ \t]*(.*)$/m);
    if (!match) {
      if (uses) continue; // a reusable-workflow call picks its runners itself
      throw new Error(`job "${key}" has no runs-on`);
    }
    const value = match[1].replace(/\s+#.*$/, "").trim();
    if (!HOSTED_LABEL.test(value))
      throw new Error(
        `job "${key}" runs-on "${value}" is not a GitHub-hosted label`,
      );
  }
}
